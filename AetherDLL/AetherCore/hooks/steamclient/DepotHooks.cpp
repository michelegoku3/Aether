#include "pch.h"
#include "hooks/steamclient/DepotHooks.h"

#include <algorithm>
#include <cstring>
#include <filesystem>
#include <sstream>
#include <string>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/HookManager.h"
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

// Applies configured manifest overrides to a depot vector in place.
void ApplyManifestOverrides(CUtlVector<DepotEntry>* vec, std::vector<ManifestPatchLog>& patches) {
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
        if (!ov || entry.manifestGid == ov->gid)
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

// True when Steam already has this app installed: its `appmanifest_<app>.acf`
// exists in one of the libraries.
//
// An installed app's depot set is Steam's own record of what is on disk, and its
// plan is what keeps it there. Filling that plan from the `.lua` would declare
// every depot the file lists - each language pack among them - as content that
// must exist, and the client would download the difference. The `.lua` is a
// content list, not a selection.
bool IsInstalled(AppId app) {
    if (g_state.steamInstallPath.empty())
        return false;
    const std::string name = "appmanifest_" + std::to_string(app) + ".acf";
    for (const std::string& dir : steamlibrary::SteamAppsDirs(g_state.steamInstallPath)) {
        std::error_code ec;
        if (std::filesystem::exists(winfile::ToPath(dir) / name, ec) && !ec)
            return true;
    }
    return false;
}

// Puts back the rows Steam leaves out for a managed app.
//
// Steam builds the plan from the license answer, so an app Aether owns only
// through package 0 comes back with no rows of its own: the plan keeps the
// shared redistributable rows every launcher app carries, the install dialog
// sizes the app at 0 B, and the downloader has nothing to walk. The `.lua` names
// the app's depots, so those fill the plan: the pinned gid where the `.lua` locks
// one, the AppInfo public gid where it does not, and AppInfo's size for the depot
// in both cases - a row without a size is an install the client sizes at 0 B.
//
// The table stays Steam's. Rows are appended, never moved or dropped - an
// installed app's plan carries depots whose content is on disk - and they are
// written into spare capacity or into a vector grown through the pattern table's
// CUtlMemory helper. That helper is the uint32 instantiation: it charges four
// bytes per requested element and copies the same amount per existing one, so the
// request is expressed in its unit and the rows it truncated are written back
// from a local copy. See `policy::GrowSlotsFor` for why the scale is not optional.
static_assert(sizeof(DepotEntry) == policy::kPlanRowBytes, "DepotEntry stride is proven at 0x20 (see StructGuard)");

void RefillPlanFromPins(AppId app, CUtlVector<DepotEntry>* vec) {
    if (!vec || (!vec->mem.memory && vec->size != 0))
        return;
    if (!o_CUtlMemoryGrow)
        return;

    const bool installed = IsInstalled(app);
    const std::size_t pinnedCount = luadata::IdsForApp(app).size();
    if (policy::DecideRefill(installed, static_cast<std::uint32_t>(pinnedCount)) == policy::RefillVerdict::Skip) {
        if (installed) {
            AC_LOG_INFO_ONCE(kModule, "Depot plan for app %u left as Steam built it: the app is installed.", app);
        }
        return;
    }
    // Without AppInfo there is no way to tell the app's depots from another
    // app's, and guessing would size the install at everything the `.lua`
    // mentions. Leave the plan alone.
    if (!gamename::Ready())
        return;

    if (const auto reason = abi::guard::CheckDepotVector(vec); !abi::guard::Passed(reason)) {
        // The table is not shaped the way this build is documented to shape it:
        // leave Steam's answer alone rather than write into it.
        abi::guard::CountRejection("DepotEntry table before refill", reason);
        return;
    }

    std::vector<policy::PinnedDepot> pinned;
    for (const AppId id : luadata::IdsForApp(app)) {
        if (id == app)
            continue;
        if (const auto ov = luadata::ManifestOverrideFor(id)) {
            pinned.push_back({id, ov->gid, ov->size});
            continue;
        }
        // A `.lua` that does not pin this depot leaves the choice to the public
        // branch; AppInfo records the gid and the size the client would plan.
        if (const std::uint64_t gid = gamename::PublicDepotGid(app, id); gid != 0)
            pinned.push_back({id, gid, gamename::PublicDepotSize(app, id)});
    }
    if (pinned.empty())
        return;

    std::vector<AppId> planned;
    planned.reserve(vec->size);
    for (std::uint32_t i = 0; i < vec->size; ++i)
        planned.push_back(vec->mem.memory[i].depotId);

    std::vector<policy::PinnedDepot> rows = policy::SelectRefillRows(app, planned, pinned);
    if (rows.empty())
        return;

    // A `.lua` also pins depots another app owns - a DLC app's content, an
    // entry of a bundle. Steam plans an app only from the depots its AppInfo
    // lists, and a row for anything else makes it discard the whole answer, so
    // the candidates are narrowed to that set before anything is written.
    rows.erase(
        std::remove_if(rows.begin(), rows.end(),
                       [app](const policy::PinnedDepot& row) { return !gamename::HasPublicDepot(app, row.depotId); }),
        rows.end());
    if (rows.empty())
        return;

    // Steam's rows are kept: an installed app's plan carries depots whose
    // content is on disk, and dropping one makes the client account that content
    // away on the next validate. The pinned rows are appended instead.
    const std::uint32_t oldSize = vec->size;
    const std::uint32_t wanted = static_cast<std::uint32_t>(rows.size());
    const std::uint32_t total = oldSize + wanted;
    if (vec->mem.allocationCount < total) {
        std::vector<DepotEntry> kept;
        if (oldSize)
            kept.assign(vec->mem.memory, vec->mem.memory + oldSize);

        o_CUtlMemoryGrow(vec, static_cast<int>(policy::GrowSlotsFor(total)));
        if (!vec->mem.memory || !policy::GrowCoversRows(vec->mem.allocationCount, total)) {
            // Leave the table as Steam built it rather than a half-grown one.
            if (vec->mem.memory)
                vec->size = oldSize;
            AC_LOG_ERROR(kModule, "Depot plan refill could not reserve room for app %u.", app);
            return;
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
        // The pin carries a gid, not a size; AppInfo has the size for that
        // depot, and a zero-sized row is an install the client sizes at 0 B.
        entry.manifestSize = gamename::PublicDepotSize(app, rows[i].depotId);
        vec->mem.memory[oldSize + i] = entry;
    }
    vec->size = total;
    AC_LOG_INFO_ONCE(kModule, "Depot plan for app %u carried %u of its own depots; planned %u from the .lua pins.", app,
                     oldSize, wanted);
}

bool h_BuildDepotDependency(void* mgr, AppId app, void* cfg, CUtlVector<DepotEntry>* depots,
                            CUtlVector<DepotEntry>* shared, void* steamApp, std::uint32_t* buildId,
                            bool* betaFallback) {
    bool result = o_BuildDepotDependency(mgr, app, cfg, depots, shared, steamApp, buildId, betaFallback);

    const bool managed = luadata::IsConfigured(app);
    std::uint32_t before = depots ? depots->size : 0;
    if (result && managed) {
        RefillPlanFromPins(app, depots);
        if (depots)
            before = depots->size;
    }

    // Steam answers with an empty plan for an app whose AppInfo is mid-swap
    // (a PICS refresh drops the old depot set before the new one lands), and its
    // caller stores whatever comes back. A stored empty plan is an install with
    // nothing to download: 0 B, a commit that writes no file, and the app still
    // stamped Fully Installed - the library then shows no size and PLAY looks
    // for an executable nobody downloaded. Refusing here makes that caller keep
    // the depot config Steam already stored; the next pass, once the AppInfo has
    // landed, builds the plan properly.
    //
    // Only the app's own vector decides. The shared/dependency vector carries
    // rows of other apps (redistributables, launcher redirects), so a row there
    // cannot make an empty own list look planned.
    const policy::PlanAnswer plan{
        .ownRows = before,
        .managed = managed,
        .steamOk = result,
    };
    if (policy::DecidePlan(plan) == policy::PlanVerdict::Refuse) {
        AC_LOG_WARN_ONCE(kModule,
                         "BuildDepotDependency: app %u came back with no depots while its AppInfo was "
                         "mid-refresh; refusing the plan so Steam keeps the depot config it already stored.",
                         app);
        // The caller discards the whole answer, so the overrides and the
        // manifest prefetch below are not worth running on it: the next pass,
        // with the AppInfo in place, runs them on a real plan.
        return false;
    }

    std::vector<ManifestPatchLog> patches;
    ApplyManifestOverrides(depots, patches);
    ApplyManifestOverrides(shared, patches);
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
}

} // namespace ac::hooks
