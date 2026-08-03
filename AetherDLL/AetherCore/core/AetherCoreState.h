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

#include "network/EticketFetcher.h"
#include "core/HookManager.h"
#include "network/ManifestFetch.h"
#include "core/Settings.h"
#include "core/SteamTypes.h"
#include "hooks/ipc/PipeWatch.h"
#include "utils/IpcSpec.h"
#include "utils/TtlCache.h"

// ---------------------------------------------------------------------------
// Central application state.
//
// Architectural principle 2: ALL shared state lives here. LumaCore spread
// inline globals across many headers; AetherCore keeps a
// single struct so ownership and lifetime are obvious and testable.
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

// Subsystem Achievement/UserStats state (stats/achievement spoofing & API cache)
struct AchievementStore {
    // Donor ID cache with TTL (24h) and LRU eviction (512 entries max).
    // Positive entries: appId -> donor SteamID
    // Negative entries: appId -> 0 (no donor found, use fallback pool)
    // Thread-safe via internal shared_mutex.
    utils::TtlCache<steam::AppId, std::uint64_t> apiCache{512, std::chrono::hours(24)};
    
    // Round-robin index for fallback pool (per-app, no TTL needed).
    mutable std::shared_mutex poolMutex;
    std::unordered_map<steam::AppId, std::size_t> nextPoolIndex;
};

struct AetherCoreState {

    // ================================================================
    //  Sub-structs — each owns its own mutex and related fields
    // ================================================================

    // ---- Lua-provided data + hot-reload bookkeeping ------------------------
    // Populated during single-threaded init AND mutated later by the hot-reload
    // watcher thread, while Steam threads read them from hooks.
    // lua.mutex guards every access (shared lock for readers, unique for the
    // watcher). LumaCore left this unsynchronised — a latent data race we fix.
    struct LuaStore {
        mutable std::shared_mutex mutex;
        std::unordered_map<steam::AppId, std::string> depotKeys;        // depot -> hex key
        std::unordered_map<steam::AppId, std::uint64_t> accessTokens;   // app -> token
        std::unordered_map<std::uint64_t, ManifestOverride> manifestOverrides;  // depot -> override
        std::unordered_set<steam::AppId> ownedAppIds;
        std::unordered_set<steam::AppId> familySharedAppIds;
        std::unordered_set<steam::AppId> libraryAppIds;  // numeric .lua filename roots shown in Library
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

    // ---- ManifestFetch -----------------------------------------------------
    // Shared state for request-code HTTP fallback. Centralised here instead of
    // module-local statics so ownership/lifetime remain explicit.
    struct ManifestFetchState {
        mutable std::mutex mutex;
        std::unordered_map<std::uint64_t, std::shared_future<std::optional<std::uint64_t>>> pending;
        std::unordered_map<manifestfetch::LookupKey,
                           std::shared_future<std::optional<std::uint64_t>>,
                           manifestfetch::LookupKeyHash> inflight;
        std::unordered_map<manifestfetch::LookupKey,
                           std::uint64_t,
                           manifestfetch::LookupKeyHash> cache;
    };
    ManifestFetchState manifestFetch;

    // ---- E-ticket runtime --------------------------------------------------
    struct EticketFetchState {
        mutable std::mutex mutex;
        std::unordered_map<eticketfetch::MintKey,
                           eticketfetch::TicketPair,
                           eticketfetch::MintKeyHash> cache;
        std::atomic<std::uint64_t> mintSuccessCount{0};
        std::atomic<std::uint64_t> mintFailureCount{0};
    };
    EticketFetchState eticketFetch;

    // ---- PipeWatch ---------------------------------------------------------
    // Snapshot of process identity per IPC pipe, built from handshake traffic.
    // Key: (pid << 32) | hSteamPipe. Guarded by mutex.
    struct PipeWatchState {
        mutable std::mutex mutex;
        std::unordered_map<std::uint64_t, pipewatch::ProcessSnapshot> snapshots;
        // Last game appId that triggered a log-dedup reset. Child processes of
        // the same session share the appId and must not reset again (atomic:
        // compared from the IPC hook thread without taking snapshots' mutex).
        std::atomic<steam::AppId> lastSessionAppId{0};
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
    // Populated during Init() from per-build TOML files. After init, these
    // maps are read-only: ResolveAddress() looks up function names here.
    // Moved from file-scope globals in PatternEngine.cpp (audit 10.2).
    struct PatternEntry {
        std::string rva;
        std::string sig;
        bool hardcodedFallback = false;
    };
    using PatternIndex = std::unordered_map<std::string, PatternEntry>;
    struct PatternState {
        PatternIndex steamclient;
        PatternIndex steamui;
    };
    PatternState patterns;

    // ---- IPC spec (per-build funcHash + optional fencepost/argc overrides) --
    // Loaded from a TOML fetched alongside the pattern tables. When loaded,
    // IPCBus uses these hashes instead of the compile-time ipc_hash::* constants
    // so that IPC dispatch survives Steam client updates that shift method hashes.
    // Populated on the init thread before hooks are installed; read-only after.
    // fencepost/argc are optional metadata (0 = absent): parsed for schema
    // compatibility and used only for diagnostics (see IpcSpec.h), never to
    // block dispatch. MethodSpec is the single definition from utils/IpcSpec.h.
    struct IpcSpecState {
        bool loaded = false;
        std::unordered_map<std::string, std::uint8_t> interfaceIds; // "IFace" -> interface id
        std::unordered_map<std::string, ipcspec::MethodSpec> methods; // "IFace::Method" -> spec
    };
    IpcSpecState ipcSpec;

    // ---- Cloud gate (LicenseHooks) ------------------------------------------
    // Per-process log dedup sets for the cloud-sync gating hooks, plus the
    // total-blocks counter shown in the blocked log lines. Written by hook
    // threads at runtime; logMutex guards every access. Moved from module-local
    // globals (centralized-state audit).
    struct CloudGateState {
        mutable std::mutex logMutex;
        std::unordered_set<steam::AppId> blockedLogged;
        std::unordered_set<steam::AppId> familyLogged;
        std::unordered_set<std::uint64_t> syncBlockedLogged;
        std::uint64_t totalBlocks = 0;  // guarded by logMutex
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
    std::string steamInstallPath;   // Folder containing steam.exe
    std::string aetherCoreDir;      // <steam>\\aethercore
    std::string steamclientPath;    // <steam>\\steamclient64.dll
    std::string steamuiPath;        // <steam>\\steamui.dll
    std::string diversionPath;      // <steam>\\bin\\acoverlay.dll (our copy)
    std::string logFilePath;        // <steam>\\aethercore\\main.log
    std::string configPath;         // <steam>\\aethercore\\aethercore.toml
    std::string patternDir;         // <steam>\\aethercore\\pattern
    std::string luaDir;             // <steam>\\config\\stplug-in
    std::string payloadDllPath;     // <steam>\\AetherPayload.dll

    // ---- Pattern diagnostics ---------------------------------------------
    std::string steamclientSha;
    std::string steamuiSha;
    bool steamclientTomlFound = false;
    bool steamuiTomlFound = false;
    std::string steamclientPatternSource;  // cache | download | missing | invalid
    std::string steamuiPatternSource;

    // ---- Loaded modules ---------------------------------------------------
    HMODULE selfModule = nullptr;
    HMODULE diversionModule = nullptr;  // The hookable steamclient copy
    HMODULE steamuiModule = nullptr;

    // ---- Lifecycle --------------------------------------------------------
    HANDLE initThread = nullptr;
    std::atomic<bool> shuttingDown{false};
    std::atomic<bool> hooksInstalled{false};

    // ---- Diversion diagnostics --------------------------------------------
    std::string diversionOutcome;  // "loaded" | "copy-failed" | "load-failed" | "not-attempted"

    // ---- Steam diagnostics ------------------------------------------------
    std::string buildId;  // steam.exe!GetBootstrapperVersion (diagnostic only)

    // ---- Captured Steam object pointers -----------------------------------
    // std::atomic<void*> ensures writes made by one thread (e.g. hook callbacks
    // on the Steam IPC thread) are visible to all readers without a data race.
    // Plain void* assignments across threads are UB in C++11.
    std::atomic<void*> pCUser{nullptr};
    std::atomic<void*> pCPackageInfo{nullptr};
    std::atomic<void*> steamEngine{nullptr};
    std::atomic<void*> pConfigStoreUserLocal{nullptr};
    std::atomic<void*> pPackage0{nullptr};  // Cached package-0 PackageInfo*
    std::atomic<bool> package0Seeded{false};

    // ---- OnlineFix --------------------------------------------------------
    std::atomic<steam::AppId> onlineFixRealAppId{0};

    // ---- Presence runtime -------------------------------------------------
    // Wire-level friends/UI presence (GamesPlayed track + PersonaState inject).
    // Decoupled from GetAppIDForCurrentPipe / session identity (see plan
    // docs/03-presence-identity-plan.md and commit 9aa4a76). Guarded by mutex.
    struct PresenceRuntime {
        mutable std::mutex mutex;
        steam::AppId playingAppId = 0;      // I_presence driver (0 = none)
        std::uint64_t selfSteamId = 0;

        std::vector<std::uint8_t> selfHdr;
        std::vector<std::uint8_t> selfBody;
        bool haveSelfTemplate = false;

        std::vector<std::uint8_t> stagedPacket;
        bool injectPending = false;

        std::unordered_map<steam::AppId,
                           std::vector<std::pair<std::string, std::string>>> rpKvs;

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

    // ---- Configuration ----------------------------------------------------
    Settings settings;

    // ---- Hook manager ------------------------------------------------------
    // Centralised hook registry — was extern HookManager g_hookManager.
    // Moved here from HookManager.{h,cpp} (audit 10.3).
    HookManager hookManager;
};

// The single shared instance.
extern AetherCoreState g_state;

}  // namespace ac
