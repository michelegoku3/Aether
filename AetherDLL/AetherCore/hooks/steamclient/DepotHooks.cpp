#include "pch.h"
#include "hooks/steamclient/DepotHooks.h"

#include <cstring>
#include <sstream>
#include <string>
#include <vector>

#include "credentials/HexCodec.h"
#include "core/HookManager.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "core/SteamTypes.h"
#include "utils/SmartIdLog.h"
#include "utils/SteamKeyPaths.h"

namespace ac::hooks {
namespace {

constexpr const char* kModule = "Depot";
using namespace ac::steam;

using LoadDepotDecryptionKey_t = std::int32_t (*)(void*, std::uint32_t, char*, char*, std::uint32_t);
using BuildDepotDependency_t = bool (*)(void*, AppId, void*, CUtlVector<DepotEntry>*,
                                        CUtlVector<DepotEntry>*, void*, std::uint32_t*, bool*);

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
        if (i) os << ", ";
        os << "{depot=" << patches[i].depotId
           << ", old=" << patches[i].oldGid
           << ", new=" << patches[i].newGid << '}';
    }
    os << ']';
    return os.str();
}

std::int32_t h_LoadDepotDecryptionKey(void* self, std::uint32_t foo, char* keyName,
                                      char* keyOut, std::uint32_t keySize) {
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
    if (!vec || !vec->mem.memory || vec->size == 0) return;

    for (std::uint32_t i = 0; i < vec->size; ++i) {
        DepotEntry& entry = vec->mem.memory[i];
        auto ov = luadata::ManifestOverrideFor(entry.depotId);
        if (!ov || entry.manifestGid == ov->gid) continue;

        patches.push_back({entry.depotId, entry.manifestGid, ov->gid});
        entry.manifestGid = ov->gid;
        if (ov->size > 0) entry.manifestSize = ov->size;
    }
}

void LogSteamSelectedManifests(AppId app, const CUtlVector<DepotEntry>* vec) {
    if (!vec || !vec->mem.memory || vec->size == 0) return;
    for (std::uint32_t i = 0; i < vec->size; ++i) {
        const DepotEntry& entry = vec->mem.memory[i];
        if (entry.depotId == 0 || entry.manifestGid == 0 ||
            !luadata::HasDepot(entry.depotId)) {
            continue;
        }
        const bool overridden = luadata::ManifestOverrideFor(entry.depotId).has_value();
        AC_LOG_DEBUG_ONCE(
            kModule,
            "Steam manifest selected app=%u depot=%u gid=%llu override=%d; no network fetch at dependency-build stage.",
            app,
            entry.depotId,
            static_cast<unsigned long long>(entry.manifestGid),
            overridden ? 1 : 0);
    }
}

bool h_BuildDepotDependency(void* mgr, AppId app, void* cfg, CUtlVector<DepotEntry>* depots,
                            CUtlVector<DepotEntry>* shared, void* steamApp,
                            std::uint32_t* buildId, bool* betaFallback) {
    bool result = o_BuildDepotDependency(mgr, app, cfg, depots, shared, steamApp, buildId,
                                         betaFallback);
    std::vector<ManifestPatchLog> patches;
    ApplyManifestOverrides(depots, patches);
    ApplyManifestOverrides(shared, patches);
    if (!patches.empty()) {
        // Once per unique line (app + patch set) per game session — the
        // logger's dedup replaces the old hidden function-local static set.
        AC_LOG_INFO_ONCE(kModule, "Manifest overrides for app %u: %s.", app,
                         ManifestPatchArray(patches).c_str());
    }

    // Observation only. Network access is deliberately owned by the concrete
    // GetManifestRequestCode path in ManifestBridge, never by this broad
    // dependency-construction hook.
    LogSteamSelectedManifests(app, depots);
    LogSteamSelectedManifests(app, shared);
    return result;
}

}  // namespace

void RegisterDepotHooks(HMODULE diversion) {
    if (!diversion) {
        AC_LOG_ERROR(kModule, "Diversion module not loaded.");
        return;
    }
    AC_LOG_INFO(kModule, "Registering depot hooks.");
    g_state.hookManager.TryHook("LoadDepotDecryptionKey", "steamclient", diversion,
                          o_LoadDepotDecryptionKey, h_LoadDepotDecryptionKey);
    g_state.hookManager.TryHook("BuildDepotDependency", "steamclient", diversion,
                          o_BuildDepotDependency, h_BuildDepotDependency);
}

}  // namespace ac::hooks
