#include "pch.h"
#include "hooks/steamclient/DepotHooks.h"

#include <cstring>
#include <sstream>
#include <string>
#include <vector>

#include "core/HookManager.h"
#include "core/Logger.h"
#include "core/SteamTypes.h"
#include "core/SteamVector.h"
#include "core/StructGuard.h"
#include "credentials/HexCodec.h"
#include "hooks/steamclient/DepotPlan.h"
#include "network/ManifestFetch.h"
#include "scripting/LuaData.h"
#include "utils/LogBurstBudget.h"
#include "utils/SmartIdLog.h"
#include "utils/SteamKeyPaths.h"

namespace ac::hooks {
namespace {

constexpr const char* kModule = "Depot";
using namespace ac::steam;

using LoadDepotDecryptionKey_t = std::int32_t (*)(void*, std::uint32_t, char*, char*, std::uint32_t);
using BuildDepotDependency_t = bool (*)(void*, AppId, void*, CUtlVector<DepotEntry>*, CUtlVector<DepotEntry>*, void*,
                                        std::uint32_t*, bool*);

LoadDepotDecryptionKey_t o_LoadDepotDecryptionKey = nullptr;
BuildDepotDependency_t o_BuildDepotDependency = nullptr;

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

// Steam plans depot rows only for the depots its AppInfo lists, and an app
// installed from a .lua has no AppInfo. Its plan therefore holds just the
// shared/dependency rows: the 251 MB instead of 118 GB case. The .lua pins are
// the only complete depot list such an app has, so they are appended to the
// plan Steam just built.
void CollectDepotIds(const CUtlVector<DepotEntry>* vec, std::vector<AppId>& out) {
    if (!vec || !vec->mem.memory)
        return;
    for (std::uint32_t i = 0; i < vec->size; ++i)
        out.push_back(vec->mem.memory[i].depotId);
}

bool AppendDepotRows(CUtlVector<DepotEntry>* vec, AppId app, const std::vector<luadata::DepotPin>& missing) {
    if (!vec || missing.empty())
        return false;
    if (const auto reason = abi::guard::CheckDepotVector(vec); !abi::guard::Passed(reason)) {
        abi::guard::CountRejection("DepotEntry table", reason);
        return false;
    }
    const std::uint32_t needed = vec->size + static_cast<std::uint32_t>(missing.size());
    if (!steamvec::Reserve(*vec, needed))
        return false;
    if (!vec->mem.memory || vec->mem.allocationCount < needed) {
        AC_LOG_WARN(kModule, "Depot plan for app %u: grow left %u slots for %u rows.", app, vec->mem.allocationCount,
                    needed);
        return false;
    }
    for (const auto& pin : missing) {
        DepotEntry& entry = vec->mem.memory[vec->size];
        entry = DepotEntry{};
        entry.depotId = pin.depotId;
        entry.appId = app;
        entry.manifestGid = pin.gid;
        entry.manifestSize = pin.size;
        ++vec->size;
    }
    if (const auto reason = abi::guard::CheckDepotVector(vec); !abi::guard::Passed(reason)) {
        abi::guard::CountRejection("DepotEntry table", reason);
        return false;
    }
    return true;
}

bool h_BuildDepotDependency(void* mgr, AppId app, void* cfg, CUtlVector<DepotEntry>* depots,
                            CUtlVector<DepotEntry>* shared, void* steamApp, std::uint32_t* buildId,
                            bool* betaFallback) {
    bool result = o_BuildDepotDependency(mgr, app, cfg, depots, shared, steamApp, buildId, betaFallback);
    std::vector<ManifestPatchLog> patches;
    ApplyManifestOverrides(depots, patches);
    ApplyManifestOverrides(shared, patches);

    // Only for apps Aether actually configures: a real Steam app keeps the plan
    // Steam built. Rows are appended before the logging and acquisition passes
    // so the pinned manifests route through the same pipeline.
    if (luadata::HasDepot(app)) {
        std::vector<AppId> present;
        CollectDepotIds(depots, present);
        CollectDepotIds(shared, present);
        const auto missing = ComputeMissingPins(luadata::PinnedDepotsForApp(app), present);
        if (!missing.empty() && !AppendDepotRows(depots, app, missing)) {
            AC_LOG_WARN(kModule, "Depot plan for app %u: could not grow Steam's depot table.", app);
        }
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

void RegisterDepotHooks(HMODULE diversion) {
    if (!diversion) {
        AC_LOG_ERROR(kModule, "Diversion module not loaded.");
        return;
    }
    AC_LOG_INFO(kModule, "Registering depot hooks.");
    g_state.hookManager.TryHook("LoadDepotDecryptionKey", "steamclient", diversion, o_LoadDepotDecryptionKey,
                                h_LoadDepotDecryptionKey);
    g_state.hookManager.TryHook("BuildDepotDependency", "steamclient", diversion, o_BuildDepotDependency,
                                h_BuildDepotDependency);
}

} // namespace ac::hooks
