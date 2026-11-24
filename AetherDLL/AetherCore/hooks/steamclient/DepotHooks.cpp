#include "pch.h"
#include "hooks/steamclient/DepotHooks.h"

#include <algorithm>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <sstream>
#include <string>
#include <unordered_set>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/HookManager.h"
#include "core/InstalledDepotRepair.h"
#include "core/Logger.h"
#include "core/SteamTypes.h"
#include "core/StructGuard.h"
#include "credentials/HexCodec.h"
#include "hooks/steamclient/DepotPlanPolicy.h"
#include "network/ManifestFetch.h"
#include "scripting/LuaData.h"
#include "utils/GameNameResolver.h"
#include "utils/LogBurstBudget.h"
#include "utils/PatternEngine.h"
#include "utils/SmartIdLog.h"
#include "utils/SteamKeyPaths.h"
#include "utils/SteamLibraryPaths.h"
#include "utils/WinFile.h"

namespace ac::hooks {
namespace {

constexpr const char* kModule = "Depot";
using namespace ac::steam;
namespace policy = ac::hooks::steamclient::policy;

using LoadDepotDecryptionKey_t = std::int32_t (*)(void*, std::uint32_t, char*, char*, std::uint32_t);
using BuildDepotDependency_t = bool (*)(void*, AppId, void*, CUtlVector<DepotEntry>*, CUtlVector<DepotEntry>*, void*,
                                        std::uint32_t*, bool*);

LoadDepotDecryptionKey_t o_LoadDepotDecryptionKey = nullptr;
BuildDepotDependency_t o_BuildDepotDependency = nullptr;
// steamclient!CUtlMemory grow helper, used to extend a plan Steam returned with
// too few rows. Resolved once at hook registration: the pattern lookup takes a
// lock and allocates, which is not safe on the Steam worker thread this hook
// runs on.
using CUtlMemoryGrow_t = void (*)(void*, int);
CUtlMemoryGrow_t o_CUtlMemoryGrow = nullptr;

logutil::SmartIdLog s_keyLog(kModule, "Injected depot keys");

struct ManifestPatchLog {
    std::uint32_t depotId = 0;
    std::uint64_t oldGid = 0;
    std::uint64_t newGid = 0;
};

std::string ManifestPatchArray(const std::vector<ManifestPatchLog>& patches) {
    std::ostringstream os;
    os << '[';
    for (std::size_t i = 0; i < patches.size(); ++i) {
        if (i)
            os << ", ";
        os << "{depot=" << patches[i].depotId << ", old=" << patches[i].oldGid << ", new=" << patches[i].newGid << '}';
    }
    os << ']';
    return os.str();
}

std::int32_t h_LoadDepotDecryptionKey(void* self, std::uint32_t foo, char* keyName, char* keyOut,
                                      std::uint32_t keySize) {
    if (auto depot = keypath::DepotIdFromDecryptionKeyName(keyName)) {
        if (auto hexKey = luadata::DepotKeyHex(*depot)) {
            if (auto bytes = ac::hex::Decode(*hexKey); bytes && bytes->size() <= keySize) {
                std::memcpy(keyOut, bytes->data(), bytes->size());
                s_keyLog.Record(*depot);
                return static_cast<std::int32_t>(bytes->size());
            }
        }
    }
    return o_LoadDepotDecryptionKey(self, foo, keyName, keyOut, keySize);
}

// The gid the appmanifest records for a depot, or 0 when it records none.
std::uint64_t RecordedGid(const std::vector<policy::RecordedDepot>& recorded, AppId depot) {
    const auto it = std::find_if(recorded.begin(), recorded.end(),
                                 [depot](const policy::RecordedDepot& row) { return row.depotId == depot; });
    return it != recorded.end() ? it->gid : 0;
}

// Applies configured manifest overrides to a depot vector in place.
//
// `recorded` is the app's installed record, or null for the shared/dependency
// vector, whose rows belong to other apps. A pinned depot Steam carried over as
// an installed-only row is promoted to a target row in the same pass, because
// the update job compares target rows only: a pin left on an installed-only row
// is mounted and recorded as installed without anything being downloaded. See
// `policy::PromotePinnedTarget`.
void ApplyManifestOverrides(CUtlVector<DepotEntry>* vec, const std::vector<policy::RecordedDepot>* recorded,
                            std::vector<ManifestPatchLog>& patches, std::vector<ManifestPatchLog>& promoted) {
    if (!vec || !vec->mem.memory || vec->size == 0)
        return;

    // Guard before the only place Aether writes into Steam's depot table.
    // Replacing a manifest id at the wrong offset does not crash: Steam
    // downloads the wrong content, or acts on a table we corrupted. One pass
    // over the rows is cheap and turns that into a skipped override.
    if (const auto reason = abi::guard::CheckDepotVector(vec); !abi::guard::Passed(reason)) {
        abi::guard::CountRejection("DepotEntry table", reason);
        return;
    }

    for (std::uint32_t i = 0; i < vec->size; ++i) {
        DepotEntry& entry = vec->mem.memory[i];
        auto ov = luadata::ManifestOverrideFor(entry.depotId);
        if (!ov)
            continue;

        const std::uint64_t recordedGid = recorded ? RecordedGid(*recorded, entry.depotId) : 0;
        if (policy::PromotePinnedTarget(ov->gid, recordedGid, entry.notNewTarget != 0)) {
            entry.notNewTarget = 0;
            promoted.push_back({entry.depotId, recordedGid, ov->gid});
        }
        if (entry.manifestGid == ov->gid)
            continue;

        patches.push_back({entry.depotId, entry.manifestGid, ov->gid});
        entry.manifestGid = ov->gid;
        if (ov->size > 0)
            entry.manifestSize = ov->size;
    }
}

void LogSteamSelectedManifests(AppId app, const CUtlVector<DepotEntry>* vec) {
    if (!log::Enabled(LogLevel::Debug))
        return;
    static logutil::LogBurstBudget detailBudget; // private logging infrastructure
    if (!vec || !vec->mem.memory || vec->size == 0)
        return;
    for (std::uint32_t i = 0; i < vec->size; ++i) {
        const DepotEntry& entry = vec->mem.memory[i];
        if (entry.depotId == 0 || entry.manifestGid == 0 || !luadata::HasDepot(entry.depotId)) {
            continue;
        }
        const auto decision = detailBudget.Admit();
        if (decision.suppressed) {
            AC_LOG_DEBUG(kModule, "Manifest detail budget: skipped %llu attempts in previous window (20/10s).",
                         static_cast<unsigned long long>(decision.suppressed));
        }
        if (!decision.emit)
            continue;
        const bool overridden = luadata::ManifestOverrideFor(entry.depotId).has_value();
        AC_LOG_DEBUG_ONCE(
            kModule,
            "Steam manifest selected app=%u depot=%u gid=%llu override=%d; routed to the local/Hubcap pipeline.", app,
            entry.depotId, static_cast<unsigned long long>(entry.manifestGid), overridden ? 1 : 0);
    }
}

// Routes every manifest Steam just selected for a managed depot into the
// acquisition pipeline: depotcache/config\depotcache and the AetherData
// backup first (a backup hit is published into depotcache), then an async
// Hubcap generation for what is really missing. Never blocks: this hook runs
// on a Steam worker thread, and Steam's own retry picks the file up once it
// has landed in depotcache.
void EnsureSelectedManifests(AppId app, const CUtlVector<DepotEntry>* vec) {
    if (!vec || !vec->mem.memory || vec->size == 0)
        return;
    for (std::uint32_t i = 0; i < vec->size; ++i) {
        const DepotEntry& entry = vec->mem.memory[i];
        if (entry.depotId == 0 || entry.manifestGid == 0 || !luadata::HasDepot(entry.depotId)) {
            continue;
        }
        manifestfetch::EnsureManifestAvailable(app, entry.depotId, entry.manifestGid);
    }
}

// An appmanifest is a few KB; anything larger is not one, and the hook runs on
// a Steam worker thread.
constexpr std::uintmax_t kMaxAcfBytes = 1u << 20;

// A dependency build is logged on one line, so a plan of a big app cannot turn
// one entry into an unbounded message.
constexpr std::uint32_t kMaxLoggedRows = 16;

bool ReadAcfBytes(const std::string& utf8Path, std::string& out) {
    std::error_code ec;
    const std::filesystem::path path = winfile::ToPath(utf8Path);
    const std::uintmax_t size = std::filesystem::file_size(path, ec);
    if (ec || size == 0 || size > kMaxAcfBytes)
        return false;
    std::ifstream input(path, std::ios::binary);
    if (!input.is_open())
        return false;
    out.assign(std::istreambuf_iterator<char>(input), std::istreambuf_iterator<char>());
    return out.size() == size;
}

// The depots the app's own appmanifest records under `InstalledDepots`, with
// the gid and the size Steam accounts them at.
//
// This is the record of what the client installed; it is not the same thing as
// "an ACF exists". Steam writes that file at the start of an update, so for a
// few seconds the app looks installed while its plan is still empty, and a plan
// without the recorded depots makes Steam remove them and delete their files.
// Empty when there is no ACF, it is unreadable, or it lists no depot: the
// caller then plans from the `.lua` pins.
std::vector<policy::RecordedDepot> RecordedDepots(AppId app) {
    std::vector<policy::RecordedDepot> recorded;
    if (g_state.steamInstallPath.empty())
        return recorded;

    const std::string name = "appmanifest_" + std::to_string(app) + ".acf";
    for (const std::string& dir : steamlibrary::SteamAppsDirs(g_state.steamInstallPath)) {
        std::string content;
        const std::string path = ac::strings::Narrow((winfile::ToPath(dir) / name).wstring());
        if (!ReadAcfBytes(path, content)) {
            if (winfile::Exists(path)) {
                AC_LOG_WARN(kModule, "App %u: appmanifest is present but unreadable; ignored in this library.", app);
            }
            continue;
        }

        const appmanifest::SplitContent split = appmanifest::SplitContentLines(content);
        // A file Steam is rewriting parses its entries above the cut, and taking
        // that prefix as the record would narrow the plan below what is on disk.
        if (!appmanifest::IsCompleteDocument(split.lines)) {
            AC_LOG_DEBUG(kModule, "App %u: appmanifest is incomplete (mid-write?); ignored in this library.", app);
            continue;
        }
        // The file name names the app; a record that names another one is not
        // this app's content, whatever the folder it was found in.
        if (!appmanifest::IsOwnRecord(content, app)) {
            AC_LOG_DEBUG(kModule, "App %u: appmanifest names another app; ignored.", app);
            continue;
        }

        for (const appmanifest::DepotEntry& entry : appmanifest::ParseInstalledDepots(split.lines)) {
            const std::uint64_t gid = appmanifest::ParseGid(entry.manifestGid);
            if (entry.depotId == 0 || gid == 0)
                continue;
            recorded.push_back(policy::RecordedDepot{entry.depotId, gid, entry.hasSize ? entry.size : 0});
        }
        if (!recorded.empty())
            break;
    }
    return recorded;
}

// One line per dependency build, so a support log can tell the calls apart.
//
// Steam builds an app's depot dependency more than once: the acquisition target
// and the installed/comparison side. A pin may be applied to only the first, or
// Steam compares the pinned gid with itself, commits a zero-file update and
// stamps that gid as installed. The arguments identify the call, and `inst`
// marks a row still carrying the gid the appmanifest records.
void LogDependencyCall(AppId app, const void* mgr, const void* cfg, const void* steamApp,
                       const CUtlVector<DepotEntry>* depots, const CUtlVector<DepotEntry>* shared,
                       const std::uint32_t* buildId, const bool* betaFallback,
                       const std::vector<policy::RecordedDepot>& recorded) {
    if (!log::Enabled(LogLevel::Debug))
        return;

    std::ostringstream os;
    os << "Dependency build app=" << app << " ctx=" << mgr << " cfg=" << cfg << " steam_app=" << steamApp
       << " own=" << (depots ? depots->size : 0) << " shared=" << (shared ? shared->size : 0)
       << " build_id=" << (buildId ? std::to_string(*buildId) : std::string("null"))
       << " beta_fallback=" << (betaFallback ? (*betaFallback ? "1" : "0") : "null") << " rows=[";
    const std::uint32_t rows = depots ? depots->size : 0;
    if (depots && depots->mem.memory) {
        const std::uint32_t shown = rows < kMaxLoggedRows ? rows : kMaxLoggedRows;
        for (std::uint32_t i = 0; i < shown; ++i) {
            const DepotEntry& entry = depots->mem.memory[i];
            if (i)
                os << ", ";
            os << entry.depotId << ':' << entry.manifestGid << ":n" << static_cast<unsigned>(entry.notNewTarget) << "s"
               << static_cast<unsigned>(entry.sharedInstall);
            if (const std::uint64_t installed = RecordedGid(recorded, entry.depotId);
                installed != 0 && installed == entry.manifestGid) {
                os << ":inst";
            }
            if (const auto pin = luadata::ManifestOverrideFor(entry.depotId); pin && pin->gid == entry.manifestGid)
                os << ":pin";
        }
        if (rows > shown)
            os << ", +" << (rows - shown) << " row(s)";
    }
    os << ']';
    AC_LOG_DEBUG(kModule, "%s", os.str().c_str());
}

// Puts back the rows Steam leaves out for a managed app.
//
// Steam builds the plan from the license answer, so an app Aether owns only
// through package 0 comes back with no rows of its own: the plan keeps the
// shared redistributable rows every launcher app carries, the install dialog
// sizes the app at 0 B, and the downloader has nothing to walk. What goes back
// depends on what the app already has: a not-installed app is planned from the
// `.lua` pins (the pinned gid where the file locks one, AppInfo's public gid and
// size where it does not); an installed app is restored from the depots its
// appmanifest records, so an empty answer cannot make the client remove a depot
// whose content is on disk and delete its files.
//
// The table stays Steam's. Rows are appended, never moved or dropped, and they
// are written into spare capacity or into a vector grown through the pattern
// table's CUtlMemory helper. That helper is the uint32 instantiation: it charges
// four bytes per requested element and copies the same amount per existing one,
// so the request is expressed in its unit and the rows it truncated are written
// back from a local copy. See `policy::GrowSlotsFor` for why the scale is not
// optional.
static_assert(sizeof(DepotEntry) == policy::kPlanRowBytes, "DepotEntry stride is proven at 0x20 (see StructGuard)");

// `true` when the plan no longer misses anything Aether knows the app has;
// `false` when rows were needed and could not be appended, so the caller must
// refuse the answer rather than store a plan that omits a recorded depot.
bool RefillDepotPlan(AppId app, const std::vector<policy::RecordedDepot>& recorded, CUtlVector<DepotEntry>* vec) {
    if (!vec || (!vec->mem.memory && vec->size != 0))
        return true;

    const std::vector<AppId> ids = luadata::IdsForApp(app);
    const policy::RefillSource source = policy::DecideRefill(!recorded.empty(), static_cast<std::uint32_t>(ids.size()),
                                                             luadata::IsRealFamilyShare(app));
    if (source == policy::RefillSource::None)
        return true;
    const bool recordedSource = source == policy::RefillSource::Recorded;

    if (const auto reason = abi::guard::CheckDepotVector(vec); !abi::guard::Passed(reason)) {
        // The table is not shaped the way this build is documented to shape it:
        // leave Steam's answer alone rather than write into it.
        abi::guard::CountRejection("DepotEntry table before refill", reason);
        return !recordedSource;
    }

    std::vector<AppId> planned;
    planned.reserve(vec->size);
    for (std::uint32_t i = 0; i < vec->size; ++i)
        planned.push_back(vec->mem.memory[i].depotId);

    // The `.lua` pins select the build: an override carries the gid and size the
    // user picked, and both sources use it.
    std::vector<policy::PinnedDepot> pinned;
    std::unordered_set<AppId> pinnedIds;
    for (const AppId id : ids) {
        if (id == app)
            continue;
        if (const auto ov = luadata::ManifestOverrideFor(id)) {
            pinned.push_back({id, ov->gid, ov->size});
            pinnedIds.insert(id);
        }
    }

    std::vector<policy::PinnedDepot> rows;
    if (recordedSource) {
        // The app is installed: restore exactly the depots its ACF records, so
        // a plan Steam answered empty cannot make the client remove one and
        // delete its content.
        rows = policy::SelectRecordedRows(app, planned, pinned, recorded);
    } else {
        // Not installed: the `.lua` pins are the only description of the app.
        // Without AppInfo there is no way to tell its depots from another app's,
        // and guessing would size the install at everything the `.lua` mentions.
        if (!gamename::Ready())
            return true;
        for (const AppId id : ids) {
            if (id == app || pinnedIds.count(id))
                continue;
            // A `.lua` that does not pin this depot leaves the choice to the
            // public branch; AppInfo records the gid and the size the client
            // would plan.
            if (const std::uint64_t gid = gamename::PublicDepotGid(app, id); gid != 0)
                pinned.push_back({id, gid, gamename::PublicDepotSize(app, id)});
        }
        if (pinned.empty())
            return true;

        rows = policy::SelectRefillRows(app, planned, pinned);
        if (rows.empty())
            return true;

        // A `.lua` also pins depots another app owns - a DLC app's content, an
        // entry of a bundle. Steam plans an app only from the depots its AppInfo
        // lists, and a row for anything else makes it discard the whole answer,
        // so the candidates are narrowed to that set before anything is written.
        rows.erase(std::remove_if(
                       rows.begin(), rows.end(),
                       [app](const policy::PinnedDepot& row) { return !gamename::HasPublicDepot(app, row.depotId); }),
                   rows.end());
    }
    // A `.lua` lists every depot the store has. A depot whose AppInfo names
    // only another platform ships the same file paths as the one this client
    // runs, so the two builds would overwrite each other's files: it is kept
    // out of the rows Aether appends. Steam's own rows are never filtered.
    std::vector<policy::PinnedDepot> keptRows;
    std::vector<AppId> foreign;
    keptRows.reserve(rows.size());
    for (const policy::PinnedDepot& row : rows) {
        if (gamename::DepotAppliesToHostOs(app, row.depotId))
            keptRows.push_back(row);
        else
            foreign.push_back(row.depotId);
    }
    rows.swap(keptRows);
    if (!foreign.empty()) {
        std::ostringstream os;
        for (const AppId id : foreign)
            os << ' ' << id;
        AC_LOG_INFO(kModule, "Depot plan for app %u leaves out depot(s) another platform runs:%s", app,
                    os.str().c_str());
    }
    if (rows.empty())
        return true;

    // Steam's rows are kept: an installed app's plan carries depots whose
    // content is on disk, and dropping one makes the client account that content
    // away on the next validate. The pinned rows are appended instead.
    const std::uint32_t oldSize = vec->size;
    const std::uint32_t wanted = static_cast<std::uint32_t>(rows.size());
    const std::uint32_t total = oldSize + wanted;
    if (vec->mem.allocationCount < total) {
        if (!o_CUtlMemoryGrow) {
            // Without the helper the rows cannot be reserved; a plan that omits
            // a recorded depot must not be stored, so the caller refuses it.
            AC_LOG_ERROR(kModule, "Depot plan refill for app %u has no CUtlMemoryGrow.", app);
            return !recordedSource;
        }
        std::vector<DepotEntry> kept;
        if (oldSize)
            kept.assign(vec->mem.memory, vec->mem.memory + oldSize);

        o_CUtlMemoryGrow(vec, static_cast<int>(policy::GrowSlotsFor(total)));
        if (!vec->mem.memory || !policy::GrowCoversRows(vec->mem.allocationCount, total)) {
            // Leave the table as Steam built it rather than a half-grown one.
            if (vec->mem.memory)
                vec->size = oldSize;
            AC_LOG_ERROR(kModule, "Depot plan refill could not reserve room for app %u.", app);
            return !recordedSource;
        }
        // The helper counted uint32 slots on both sides: it reserved four bytes
        // per slot, put the requested count on the tail and copied four bytes
        // per existing row. Undo the tail, express the capacity in rows and
        // write the rows it truncated back.
        vec->size = oldSize;
        vec->mem.allocationCount = total;
        for (std::uint32_t i = 0; i < oldSize; ++i)
            vec->mem.memory[i] = kept[i];
    }

    for (std::uint32_t i = 0; i < wanted; ++i) {
        DepotEntry entry{};
        entry.depotId = rows[i].depotId;
        entry.appId = app;
        entry.manifestGid = rows[i].gid;
        // The pin's size describes the build it selected, the ACF's size what is
        // installed, and AppInfo's size the public branch: a zero-sized row is an
        // install the client sizes at 0 B, so the first known value is used.
        entry.manifestSize = policy::RowSize(rows[i], gamename::PublicDepotSize(app, rows[i].depotId));
        vec->mem.memory[oldSize + i] = entry;
    }
    vec->size = total;
    if (recordedSource) {
        AC_LOG_INFO_ONCE(kModule,
                         "Depot plan for app %u carried %u of its own depots; restored %u the appmanifest records.",
                         app, oldSize, wanted);
    } else {
        AC_LOG_INFO_ONCE(kModule, "Depot plan for app %u carried %u of its own depots; planned %u from the .lua pins.",
                         app, oldSize, wanted);
    }
    return true;
}

bool h_BuildDepotDependency(void* mgr, AppId app, void* cfg, CUtlVector<DepotEntry>* depots,
                            CUtlVector<DepotEntry>* shared, void* steamApp, std::uint32_t* buildId,
                            bool* betaFallback) {
    // The app manager every plan is built for. The update check Aether asks for
    // when a pin moves is a method of this object, and this hook is the only
    // place the DLL is handed it.
    g_state.overview.clientAppManager.store(mgr, std::memory_order_release);
    bool result = o_BuildDepotDependency(mgr, app, cfg, depots, shared, steamApp, buildId, betaFallback);

    const bool managed = luadata::IsConfigured(app);
    // One read of the installed record per call: the log, the refill and the
    // promotion all look at the same appmanifest.
    const std::vector<policy::RecordedDepot> recorded =
        managed ? RecordedDepots(app) : std::vector<policy::RecordedDepot>{};
    if (managed)
        LogDependencyCall(app, mgr, cfg, steamApp, depots, shared, buildId, betaFallback, recorded);
    std::uint32_t before = depots ? depots->size : 0;
    bool whole = true;
    if (result && managed) {
        whole = RefillDepotPlan(app, recorded, depots);
        if (depots)
            before = depots->size;
    }

    // A managed app whose own vector is still empty after the refill: the plan
    // is what the caller stores, and an empty one is an install with nothing to
    // download - 0 B, a commit that writes no file, and the app still stamped
    // Fully Installed. The library then shows no size and PLAY looks for an
    // executable nobody downloaded. The AppInfo being mid-swap is the common
    // cause (a PICS refresh drops the old depot set before the new one lands),
    // but an ACF that cannot be read or names no usable depot ends the same way.
    // Refusing here makes that caller keep the depot config Steam already
    // stored; the next pass plans the app again once the data it needs is there.
    //
    // Only the app's own vector decides. The shared/dependency vector carries
    // rows of other apps (redistributables, launcher redirects), so a row there
    // cannot make an empty own list look planned.
    const policy::PlanAnswer plan{
        .ownRows = before,
        .managed = managed,
        .steamOk = result,
    };
    if (!whole) {
        AC_LOG_WARN_ONCE(kModule,
                         "BuildDepotDependency: app %u still misses a depot its appmanifest records; refusing the "
                         "plan so Steam keeps the depot config it already stored.",
                         app);
        return false;
    }
    if (policy::DecidePlan(plan) == policy::PlanVerdict::Refuse) {
        AC_LOG_WARN_ONCE(kModule,
                         "BuildDepotDependency: app %u answered with no depot rows and the refill had nothing usable "
                         "to put back; refusing the plan so Steam keeps the depot config it already stored.",
                         app);
        // The caller discards the whole answer, so the overrides and the
        // manifest prefetch below are not worth running on it: the next pass,
        // with the AppInfo in place, runs them on a real plan.
        return false;
    }

    std::vector<ManifestPatchLog> patches;
    std::vector<ManifestPatchLog> promoted;
    ApplyManifestOverrides(depots, &recorded, patches, promoted);
    ApplyManifestOverrides(shared, nullptr, patches, promoted);
    if (!promoted.empty()) {
        AC_LOG_INFO_ONCE(kModule, "Pinned depots made update targets for app %u: %s.", app,
                         ManifestPatchArray(promoted).c_str());
    }
    if (!patches.empty()) {
        // Once per unique line (app + patch set) per game session — the
        // logger's dedup handles the suppression.
        AC_LOG_INFO_ONCE(kModule, "Manifest overrides for app %u: %s.", app, ManifestPatchArray(patches).c_str());
    }

    // Proactive trigger: route every managed depot/GID straight into the
    // local-first/Hubcap pipeline so the exact manifest is already on disk when
    // Steam asks for a request code (hooks/wire/ManifestBridge.cpp). The
    // per-depot call is cheap when the manifest exists and async when it not.
    LogSteamSelectedManifests(app, depots);
    LogSteamSelectedManifests(app, shared);
    EnsureSelectedManifests(app, depots);
    EnsureSelectedManifests(app, shared);
    return result;
}

} // namespace

void RegisterDepotHooks(HMODULE steamclient) {
    if (!steamclient) {
        AC_LOG_ERROR(kModule, "Steamclient module not loaded.");
        return;
    }
    AC_LOG_INFO(kModule, "Registering depot hooks.");
    o_CUtlMemoryGrow =
        reinterpret_cast<CUtlMemoryGrow_t>(pattern::ResolveAddress("CUtlMemoryGrow", "steamclient", steamclient));
    g_state.hookManager.TryHook("LoadDepotDecryptionKey", "steamclient", steamclient, o_LoadDepotDecryptionKey,
                                h_LoadDepotDecryptionKey);
    g_state.hookManager.TryHook("BuildDepotDependency", "steamclient", steamclient, o_BuildDepotDependency,
                                h_BuildDepotDependency);

    // The per-app update check the launch path calls. Optional on its own: a
    // table without it leaves the card on the action of the state Steam last
    // computed (see AppOverviewHooks::DrainRechecks), while the pins still reach
    // the next plan.
    void* check = pattern::ResolveAddress("CheckAppForUpdate", "steamclient", steamclient);
    g_state.overview.clientCheckAppForUpdate.store(check, std::memory_order_release);
    if (check != nullptr)
        AC_LOG_INFO(kModule, "Per-app update check resolved at 0x%p.", check);
    else
        AC_LOG_WARN_ONCE(kModule, "CheckAppForUpdate unresolved: a moved pin will not offer UPDATE until Steam checks "
                                  "the app on its own.");
}

} // namespace ac::hooks
