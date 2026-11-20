#pragma once

#include <atomic>
#include <chrono>
#include <future>
#include <mutex>
#include <optional>
#include <shared_mutex>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include "core/HookManager.h"
#include "core/Settings.h"
#include "core/SteamTypes.h"
#include "hooks/ipc/PipeWatch.h"
#include "hooks/steamui/AppOverviewPolicy.h"
#include "hooks/steamui/AppRecheckPolicy.h"
#include "hooks/wire/AchievementUi.h"
#include "hooks/wire/DonorPool.h"
#include "hooks/wire/DonorProbe.h"
#include "network/EticketFetcher.h"
#include "utils/IpcSpec.h"
#include "utils/TtlCache.h"

// ---------------------------------------------------------------------------
// Central application state.
//
// Architectural principle 2: ALL shared state lives here. Keeping it in one
// struct makes ownership and lifetime obvious and testable.
//
// Fields that are read/written from multiple threads use std::atomic. Lua data
// is also mutated by the hot-reload watcher, so all Lua maps are accessed only
// through LuaData under lua.mutex.
//
// Sub-structs group related fields so each subsystem's state is self-contained
// and navigable without scrolling through 40 unrelated fields.
//
// Module-local state is admitted ONLY when it is:
//   (a) private lifecycle infrastructure of one service module (Lua
//       interpreter, DirWatch thread/control block, logger internals,
//       SmartIdLog registry);
//   (b) init-time hook plumbing immutable after hook installation (MinHook
//       trampolines o_*, resolved function pointers, dispatch tables);
//   (c) per-thread scratch state declared thread_local, therefore not shared;
//   (d) per-DLL micro-state in targets that have no AetherCoreState
//       (the payload/injector DLLs).
// Everything else — anything mutable at runtime and shared across modules or
// threads — belongs in this struct. See docs/ARCHITECTURE.md for the audit
// checklist and allowed-exception policy.
// ---------------------------------------------------------------------------
namespace ac {

// Manifest override target supplied by a Lua script.
struct ManifestOverride {
    std::uint64_t gid = 0;
    std::uint64_t size = 0;
};

// Subsystem Achievement/UserStats state: donor selection, per-app pool
// learning and attempt correlation live in DonorPool::SharedState below.
// Central-state rule: shared domain data belongs here with its own locks,
// never as module-local globals (see docs/ARCHITECTURE.md).
struct AchievementStore {
    hooks::DonorPool::SharedState donor;
    // Donor-probe walk bookkeeping (in-flight apps plus the injected wire
    // sender/schema sink). Donor learning itself stays in `donor` above.
    hooks::wire::donorprobe::State probe;
    // Client achievement page (service path): captured schemas, the local
    // .bin state read on workers, job correlation and the rewrite counters.
    achieve::State ui;
};

struct AetherCoreState {

    // ================================================================
    //  Sub-structs — each owns its own mutex and related fields
    // ================================================================

    // ---- Lua-provided data + hot-reload bookkeeping ------------------------
    // Populated during single-threaded init AND mutated later by the hot-reload
    // watcher thread, while Steam threads read them from hooks.
    // lua.mutex guards every access (shared lock for readers, unique for the
    // watcher).
    struct LuaStore {
        mutable std::shared_mutex mutex;
        std::unordered_map<steam::AppId, std::string> depotKeys;               // depot -> hex key
        std::unordered_map<steam::AppId, std::uint64_t> accessTokens;          // app -> token
        std::unordered_map<std::uint64_t, ManifestOverride> manifestOverrides; // depot -> override
        std::unordered_set<steam::AppId> ownedAppIds;
        std::unordered_set<steam::AppId> familySharedAppIds;
        // Apps whose family bit is Aether's own owner banner, not a licence.
        // They stay in familySharedAppIds (the client draws them as shared) and
        // here, so HasDepot keeps providing them: see FamilyClassification.h.
        std::unordered_set<steam::AppId> bannerFamilyAppIds;
        // Steam did not provide the app and Aether already answered as owned.
        // Package 0 makes Steam report owned afterwards; this flag keeps the
        // spoof from being reclassified as a genuine license.
        std::unordered_set<steam::AppId> spoofedAppIds;
        // Real family lender per app, captured before package 0 makes the app
        // look owned. Kept in package 0 for play, attributed to this account
        // for the banner.
        std::unordered_map<steam::AppId, std::uint32_t> familyLenderAppIds;
        std::unordered_set<steam::AppId> libraryAppIds; // numeric .lua filename roots (PICS/name lookup)
        // Auth tickets are persisted to the registry (see Ticket.{h,cpp}), not held
        // in process memory, so there are no ticket maps here.

        // Which depots / library apps / tokens / manifest overrides / eticket urls
        // each .lua file contributed. Grouped in one struct so that adding a new
        // per-file data type (e.g. forcedenuvo, addprocess) only touches one place.
        // Ref-counts are kept separate because they track cross-file cardinality.
        struct FileContributions {
            std::unordered_set<steam::AppId> depots;
            std::unordered_set<steam::AppId> libraryApps;
            std::unordered_map<steam::AppId, std::uint64_t> accessTokens;
            std::unordered_map<std::uint64_t, ManifestOverride> manifestOverrides;
            std::string eticketUrl;
        };
        std::unordered_map<std::string, FileContributions> fileContributions;
        std::unordered_map<steam::AppId, std::uint32_t> depotRefCount;
        std::unordered_map<steam::AppId, std::uint32_t> libraryRefCount;
        std::string eticketUrl;
        // Changes staged for the next NotifyLicenseChanged pass.
        std::vector<steam::AppId> pendingAdditions;
        std::vector<steam::AppId> pendingRemovals;
        // The file currently being parsed; contributions are attributed to it.
        // Same guard as the rest of LuaStore (lua.mutex, unique lock).
        std::string currentFile;
    };
    LuaStore lua;

    // ---- E-ticket runtime --------------------------------------------------
    struct EticketFetchState {
        mutable std::mutex mutex;
        std::unordered_map<eticketfetch::MintKey, eticketfetch::TicketPair, eticketfetch::MintKeyHash> cache;
        std::atomic<std::uint64_t> mintSuccessCount{0};
        std::atomic<std::uint64_t> mintFailureCount{0};
    };
    EticketFetchState eticketFetch;

    // ---- PipeWatch ---------------------------------------------------------
    // Snapshot of process identity per IPC pipe, built from handshake traffic.
    // Key: (pid << 32) | hSteamPipe. Guarded by mutex.
    // Eviction (A5): when the map reaches kPipeWatchMaxSnapshots, dead processes
    // are reaped first (OpenProcess probe), then the oldest entry is removed
    // (FIFO by capturedAt). evictionCount tracks total evictions for diagnostics.
    struct PipeWatchState {
        mutable std::mutex mutex;
        std::unordered_map<std::uint64_t, pipewatch::ProcessSnapshot> snapshots;
        // Last game appId that triggered a log-dedup reset. Child processes of
        // the same session share the appId and must not reset again (atomic:
        // compared from the IPC hook thread without taking snapshots' mutex).
        std::atomic<steam::AppId> lastSessionAppId{0};
        // Total number of snapshot evictions (dead process reaping or FIFO).
        std::atomic<std::uint64_t> evictionCount{0};
    };
    PipeWatchState pipeWatch;

    // ---- Online payload ----------------------------------------------------
    struct OnlinePayloadState {
        mutable std::mutex mutex;
        std::unordered_set<std::uint32_t> injectedPids;
        std::atomic<std::uint64_t> injectSuccessCount{0};
        std::atomic<std::uint64_t> injectFailureCount{0};
    };
    OnlinePayloadState onlinePayload;

    // ---- Pattern engine runtime index --------------------------------------
    // Populated during Init() from per-build TOML files. After init, the
    // late-pattern retry may publish missing maps. All runtime access takes mutex.
    struct PatternEntry {
        std::string rva;
        std::string sig;
    };
    using PatternIndex = std::unordered_map<std::string, PatternEntry>;
    struct PatternState {
        mutable std::shared_mutex mutex;
        PatternIndex steamclient;
        PatternIndex steamui;
    };
    PatternState patterns;

    // ---- ABI table (per-build struct offsets published with the patterns) --
    // The generator extracts these from the build's own code, so the DLL stops
    // carrying compiled offsets. Consumed by core/NetPacketAbi.h, which treats
    // them as a candidate to verify, never as a fact.
    struct AbiTableState {
        std::mutex mutex;
        // "Struct.field" -> offset, exactly as published. The two modules
        // publish different structs (steamclient: ownership/package/depot;
        // steamui: the app-overview prefix), so they are kept apart instead of
        // overwriting each other when both tables load.
        std::unordered_map<std::string, std::uint32_t> offsets;
        std::string source; // sha of the table the offsets came from
        std::unordered_map<std::string, std::uint32_t> uiOffsets;
        std::string uiSource;
    };
    AbiTableState abiTable;

    // ---- SteamUI app overview (validated write path) ----------------------
    // The probe is per-session evidence that the compiled CSteamApp prefix is
    // the live object; the counters make the layer observable when a card is
    // not repaired. `lastReject` is read by StatusWriter under
    // statusMetadataMutex, like the other published strings.
    struct OverviewState {
        std::mutex mutex; // guards probe only: two client threads can fill cards
        hooks::steamui::policy::ProbeState probe;
        std::atomic<std::uint64_t> fills{0};
        std::atomic<std::uint64_t> writes{0};
        std::atomic<std::uint64_t> rejects{0};
        std::atomic<bool> latched{false};
        std::string lastReject;

        // ---- app recheck (worker writes, SteamUI thread drains) ------------
        // A version apply rewrites the Lua on a worker, but the client only
        // re-evaluates an app from its own frame callback, so the ids wait here
        // in between. RecheckQueue is not thread-safe by design: recheckMutex is
        // the owner's guard, deliberately not `mutex` above, which latches the
        // layout probe on the card path — a drain must never wait on that.
        std::mutex recheckMutex;
        hooks::steamui::policy::RecheckQueue rechecks;
        // The app-list owner, learned from the last MarkAppChange call: a rebuilt
        // client UI or an account switch replaces the object, so keeping the
        // first one would leave a dangling pointer. Read only through this
        // atomic: the detour runs on the client's threads and Aether never
        // supplies an object of its own.
        std::atomic<void*> appChangeSource{nullptr};
        // Ids handed to the client, one MarkAppChange call each.
        std::atomic<std::uint64_t> recheckCalls{0};

        // ---- restoring a card (SteamUI thread only) ------------------------
        // The client keeps the app object after its owned bit is taken back; it
        // only drops it from the library. Putting the bit back therefore needs
        // that object, and the client hands one out only through its own lookup.
        // `appController` is the owner learned from that lookup's `self`;
        // `lookupProbe` is the evidence that the compiled prefix really is the
        // object it returns, latched on two ids Aether already filled.
        std::atomic<void*> appController{nullptr};
        std::mutex lookupMutex; // guards lookupProbe only
        hooks::steamui::policy::ProbeState lookupProbe;
        std::atomic<bool> lookupLatched{false};
        std::atomic<std::uint64_t> restores{0};

        // ---- library membership (worker retires, SteamUI thread clears) ----
        // A deleted `.lua` must take its card out of the library without a
        // restart. `tracked` records the ids Aether provided on a card fill: the
        // owned bit is usually already set by the client (package 0 makes Steam
        // answer the app as licensed), so authorship is not the question - the
        // set is what says "this card exists because of Aether, taking it back is
        // ours to do". `retired` records the ids an explicit file removal named;
        // it survives a late hook install, so a retirement decided before the
        // layer exists is still applied by the next card fill. `hidden` records
        // the ids whose bit was taken back: Steam dropped their overview object,
        // so a later `.lua` re-add has to be announced as a new app, not as a
        // config change. All three sets are bounded; a refusal is logged and
        // counted.
        std::mutex stampMutex; // guards the sets, never the probe or the queue
        std::unordered_set<steam::AppId> tracked;
        std::unordered_set<steam::AppId> retired;
        std::unordered_set<steam::AppId> hidden;
        std::atomic<std::uint64_t> clears{0};
    };
    OverviewState overview;

    // ---- IPC spec (per-build funcHash + optional fencepost/argc overrides) --
    // Loaded from a TOML fetched alongside the pattern tables. When loaded,
    // IPCBus uses these hashes instead of the compile-time ipc_hash::* constants
    // so that IPC dispatch survives Steam client updates that shift method hashes.
    // Init or late retry publishes maps once, then sets loaded (release).
    // Readers acquire loaded BEFORE touching maps; maps never change after true.
    // fencepost/argc are optional metadata (0 = absent): parsed for schema
    // compatibility and used only for diagnostics (see IpcSpec.h), never to
    // block dispatch. MethodSpec is the single definition from utils/IpcSpec.h.
    struct IpcSpecState {
        std::atomic<bool> loaded{false};
        std::unordered_map<std::string, std::uint8_t> interfaceIds;   // "IFace" -> interface id
        std::unordered_map<std::string, ipcspec::MethodSpec> methods; // "IFace::Method" -> spec
    };
    IpcSpecState ipcSpec;

    // ---- Cloud gate (LicenseHooks) ------------------------------------------
    // Per-process log dedup sets for the cloud-sync gating hooks, plus the
    // total-blocks counter shown in the blocked log lines. Written by hook
    // threads at runtime; logMutex guards every access.
    struct CloudGateState {
        mutable std::mutex logMutex;
        std::unordered_set<steam::AppId> blockedLogged;
        std::unordered_set<steam::AppId> familyLogged;
        std::unordered_set<std::uint64_t> syncBlockedLogged;
        std::uint64_t totalBlocks = 0; // guarded by logMutex
    };
    CloudGateState cloudGate;

    // ---- Game name resolver ---------------------------------------------------
    // Captured CAppInfoCache object + per-app display-name cache, used by the
    // presence pipeline. appInfoCacheObj is written by the steamclient hook
    // thread and read by wire threads — atomic because plain void* across
    // threads is UB. nameCache uses TTL cache with LRU eviction.
    struct GameNameState {
        std::atomic<void*> appInfoCacheObj{nullptr};
        // Game name cache with TTL (6h) and LRU eviction (512 entries max).
        // Positive entries: appId -> game name
        // Negative entries: appId -> "" (name not available)
        // Thread-safe via internal shared_mutex.
        utils::TtlCache<steam::AppId, std::string> nameCache{512, std::chrono::hours(6)};
    };
    GameNameState gameName;

    // ================================================================
    //  Top-level fields — not part of a cluster
    // ================================================================

    // ---- Resolved runtime paths -------------------------------------------
    std::string steamInstallPath; // Folder containing steam.exe
    std::string deskDataDir;      // Startup bridge snapshot shared by config and backups
    std::string aetherCoreDir;    // <steam>\\aethercore
    std::string steamclientPath;  // <steam>\\steamclient64.dll
    std::string steamuiPath;      // <steam>\\steamui.dll
    std::string logFilePath;      // <steam>\\aethercore\\main.log
    std::string configPath;       // <steam>\\aethercore\\aethercore.toml
    std::string patternDir;       // <steam>\\aethercore\\pattern
    std::string luaDir;           // <steam>\\config\\stplug-in
    std::string payloadDllPath;   // <steam>\\AetherPayload.dll

    // ---- Pattern diagnostics ---------------------------------------------
    // Guard mutable status strings/flags during startup and late retry.
    mutable std::mutex statusMetadataMutex;
    std::string steamclientSha;
    std::string steamuiSha;
    bool steamclientTomlFound = false;
    bool steamuiTomlFound = false;
    std::string steamclientPatternSource; // "<src>:<mirror>" (fresh fetch) |
                                          // "<src>:<mirror> (cache)" (provenance
                                          // sidecar) | cache | missing | invalid
    std::string steamuiPatternSource;

    // ---- Loaded modules ---------------------------------------------------
    HMODULE selfModule = nullptr;
    HMODULE steamclientModule = nullptr; // live steamclient64.dll hook target
    HMODULE steamuiModule = nullptr;     // live steamui.dll hook target (overview)

    // ---- Lifecycle --------------------------------------------------------
    HANDLE initThread = nullptr;
    std::atomic<bool> shuttingDown{false};
    std::atomic<bool> hooksInstalled{false};

    // ---- Steamclient attach diagnostics -----------------------------------
    // live-mapped (Steam mapped it) | live-not-mapped (timeout) | path-invalid
    std::string steamclientAttach;
    // mapped | not-mapped | path-invalid
    std::string steamuiAttach;

    // ---- Steam diagnostics ------------------------------------------------
    std::string buildId; // steam.exe!GetBootstrapperVersion (diagnostic only)
    // Selected client branch from <steam>\package\beta; empty on stable. Read
    // under statusMetadataMutex, like the other startup strings.
    std::string steamBetaBranch;

    // ---- Captured Steam object pointers -----------------------------------
    // std::atomic<void*> ensures writes made by one thread (e.g. hook callbacks
    // on the Steam IPC thread) are visible to all readers without a data race.
    // Plain void* assignments across threads are UB in C++11.
    std::atomic<void*> pCUser{nullptr};
    std::atomic<void*> pCPackageInfo{nullptr};
    std::atomic<void*> steamEngine{nullptr};
    std::atomic<void*> pConfigStoreUserLocal{nullptr};
    std::atomic<void*> pPackage0{nullptr}; // Cached package-0 PackageInfo*
    std::atomic<bool> package0Published{false};

    // ---- AetherOnline --------------------------------------------------------
    // Session identity (real app behind the 480 mask, showonline, UCO2/OFME
    // spoof flag) lives in hooks/aetheronline/PresenceSession — owned by
    // AetherOnlineHooks::h_SpawnProcess, consumed via immutable snapshot.

    // Last app Steam asked us to spawn (even in exclude/None). Used only to
    // name a later Spacewar (480) spoof from UCO2/OFME. Cleared on a 480
    // launch (direct UCO2/OFME launch, or the user playing Spacewar itself):
    // the spoofed session is then named from its live pipe image, not from an
    // earlier real game.
    std::atomic<steam::AppId> lastSpawnedAppId{0};

    // ---- Presence runtime -------------------------------------------------
    // Wire-level friends/UI presence (GamesPlayed track + PersonaState inject).
    // Decoupled from GetAppIDForCurrentPipe / session identity. Guarded by mutex.
    struct PresenceRuntime {
        mutable std::mutex mutex;
        steam::AppId playingAppId = 0; // I_presence driver (0 = none)
        std::uint64_t selfSteamId = 0;

        std::vector<std::uint8_t> selfHdr;
        std::vector<std::uint8_t> selfBody;
        bool haveSelfTemplate = false;

        std::vector<std::uint8_t> stagedPacket;
        bool injectPending = false;

        std::unordered_map<steam::AppId, std::vector<std::pair<std::string, std::string>>> rpKvs;

        std::uint64_t injectDeliverCount = 0;
        std::uint64_t injectBuildFailCount = 0;
        std::uint64_t gamesPlayedTrackCount = 0;
        std::uint64_t extraInfoPatchCount = 0;
    };
    PresenceRuntime presence;

    // ---- IPC --------------------------------------------------------------
    // RequestEncryptedAppTicket records the async-call handle here so the later
    // GetAPICallResult(EncryptedAppTicketResponse) can answer with k_EResultOK.
    // The record is bounded and timestamped; the IPC handler owns all mutation.
    struct PendingETicket {
        steam::AppId appId = 0;
        std::chrono::steady_clock::time_point createdAt{};
    };
    struct PendingETicketState {
        mutable std::mutex mutex;
        std::unordered_map<std::uint64_t, PendingETicket> entries;
        std::uint64_t recordedCount = 0;
        std::uint64_t claimedCount = 0;
        std::uint64_t expiredCount = 0;
        std::uint64_t rejectedCount = 0;
        std::uint64_t evictedCount = 0;
    };
    PendingETicketState pendingETickets;

    // ---- Config-store ticket cache ----------------------------------------
    mutable std::mutex configStoreTicketMutex;
    std::unordered_map<steam::AppId, std::vector<std::uint8_t>> configStoreAppTickets;

    // ---- Achievement / Stats Store ----------------------------------------
    AchievementStore achievements;

    // ---- Ticket forge diagnostics -----------------------------------------
    std::atomic<std::uint64_t> ticketForgeSuccessCount{0};
    std::atomic<std::uint64_t> ticketForgeFailureCount{0};

    // ---- License reload diagnostics (A4) -----------------------------------
    // Counters for m_bReloadAll forced via SendCallbackToPipe hook and direct
    // MarkLicenseAsChanged calls from LicenseManager::NotifyLicenseChanged.
    // Atomic: incremented by hook threads, read by StatusWriter.
    std::atomic<std::uint64_t> licenseReloadForcedCount{0};
    std::atomic<std::uint64_t> licenseReloadDirectCount{0};

    // ---- Hubcap key check --------------------------------------------------
    // Definitive saved-key verdict from the startup `/user/stats` check, read
    // by StatusWriter: 0 = unknown, 1 = active, 2 = inactive. The key itself is
    // never stored here.
    std::atomic<int> hubcapKeyState{0};
    std::atomic<std::int64_t> hubcapKeyCheckedEpoch{0};

    // ---- Configuration ----------------------------------------------------
    std::atomic<std::shared_ptr<const Settings>> settings{std::make_shared<const Settings>()};

    // ---- Hook manager ------------------------------------------------------
    // Centralised hook registry (single process-wide instance).
    HookManager hookManager;
};

// The single shared instance.
extern AetherCoreState g_state;

} // namespace ac
