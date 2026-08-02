#include "pch.h"
#include "scripting/LuaData.h"

#include <mutex>
#include <shared_mutex>
#include <string>
#include <unordered_set>
#include <vector>

#include "core/Logger.h"

namespace ac::luadata {
namespace {

constexpr const char* kModule = "LuaData";

using F = AetherCoreState::LuaStore::FileContributions;

// ---- Generic rebuild helpers -----------------------------------------------
// On file unload, each scalar field (non-ref-counted) must be rebuilt from the
// remaining files. The "last file wins" rule applies.
// Caller holds lua.mutex uniquely.

void RebuildScalars() {
    auto& lua = g_state.lua;
    lua.accessTokens.clear();
    lua.manifestOverrides.clear();
    lua.eticketUrl.clear();

    for (const auto& [_, c] : lua.fileContributions) {
        for (const auto& [appId, tok] : c.accessTokens)
            lua.accessTokens[appId] = tok;
        for (const auto& [depotId, ov] : c.manifestOverrides)
            lua.manifestOverrides[depotId] = ov;
        if (!c.eticketUrl.empty())
            lua.eticketUrl = c.eticketUrl;
    }
}

// ---- Ref-count helpers ------------------------------------------------------
// Dec-refs an id and, if it reaches zero, removes it from the global set and
// appends it to the pending-removals list.
// Caller holds lua.mutex uniquely.

template <typename IdSet, typename Id, typename RefCountMap>
void DecRefAndCleanup(IdSet& globalSet, RefCountMap& refCounts, const Id& id,
                      std::vector<steam::AppId>& pendingRemovals) {
    auto it = refCounts.find(id);
    if (it != refCounts.end() && --it->second == 0) {
        refCounts.erase(it);
        globalSet.erase(id);
        pendingRemovals.push_back(id);
    }
}

}  // namespace

// ============================ Reads =========================================

bool HasDepot(steam::AppId appId) {
    std::shared_lock lock(g_state.lua.mutex);
    return g_state.lua.depotKeys.count(appId) && !g_state.lua.ownedAppIds.count(appId) &&
           !g_state.lua.familySharedAppIds.count(appId);
}

bool IsConfigured(steam::AppId appId) {
    std::shared_lock lock(g_state.lua.mutex);
    return g_state.lua.depotKeys.count(appId) > 0;
}

bool IsOwned(steam::AppId appId) {
    std::shared_lock lock(g_state.lua.mutex);
    return g_state.lua.ownedAppIds.count(appId) > 0;
}

bool IsFamilyShared(steam::AppId appId) {
    std::shared_lock lock(g_state.lua.mutex);
    return g_state.lua.familySharedAppIds.count(appId) > 0;
}

std::optional<std::string> DepotKeyHex(steam::AppId depotId) {
    std::shared_lock lock(g_state.lua.mutex);
    auto it = g_state.lua.depotKeys.find(depotId);
    if (it == g_state.lua.depotKeys.end() || it->second.empty()) return std::nullopt;
    return it->second;
}

std::uint64_t AccessToken(steam::AppId appId) {
    std::shared_lock lock(g_state.lua.mutex);
    auto it = g_state.lua.accessTokens.find(appId);
    return it != g_state.lua.accessTokens.end() ? it->second : 0;
}

std::optional<ManifestOverride> ManifestOverrideFor(std::uint64_t depotId) {
    std::shared_lock lock(g_state.lua.mutex);
    auto it = g_state.lua.manifestOverrides.find(depotId);
    if (it == g_state.lua.manifestOverrides.end()) return std::nullopt;
    return it->second;
}

bool HasManifestOverrides() {
    std::shared_lock lock(g_state.lua.mutex);
    return !g_state.lua.manifestOverrides.empty();
}

std::vector<steam::AppId> AllDepotIds() {
    std::shared_lock lock(g_state.lua.mutex);
    std::vector<steam::AppId> ids;
    ids.reserve(g_state.lua.depotKeys.size());
    for (const auto& [id, _] : g_state.lua.depotKeys) ids.push_back(id);
    return ids;
}

std::vector<steam::AppId> LibraryAppIds() {
    std::shared_lock lock(g_state.lua.mutex);
    std::vector<steam::AppId> ids;
    ids.reserve(g_state.lua.libraryAppIds.size());
    for (steam::AppId id : g_state.lua.libraryAppIds) ids.push_back(id);
    return ids;
}

std::unordered_map<std::string, std::vector<steam::AppId>> ConfiguredIdsByFile() {
    std::shared_lock lock(g_state.lua.mutex);
    std::unordered_map<std::string, std::vector<steam::AppId>> out;
    out.reserve(g_state.lua.fileContributions.size());
    for (const auto& [path, c] : g_state.lua.fileContributions) {
        std::vector<steam::AppId> ids;
        ids.reserve(c.depots.size() + c.libraryApps.size());
        std::unordered_set<steam::AppId> seen;
        for (steam::AppId id : c.depots) {
            if (seen.insert(id).second) ids.push_back(id);
        }
        for (steam::AppId id : c.libraryApps) {
            if (seen.insert(id).second) ids.push_back(id);
        }
        out.emplace(path, std::move(ids));
    }
    return out;
}

std::size_t LoadedFileCount() {
    std::shared_lock lock(g_state.lua.mutex);
    return g_state.lua.fileContributions.size();
}

std::size_t ConfiguredDepotCount() {
    std::shared_lock lock(g_state.lua.mutex);
    return g_state.lua.depotKeys.size();
}

std::size_t AccessTokenCount() {
    std::shared_lock lock(g_state.lua.mutex);
    return g_state.lua.accessTokens.size();
}

std::size_t ManifestOverrideCount() {
    std::shared_lock lock(g_state.lua.mutex);
    return g_state.lua.manifestOverrides.size();
}

std::string EticketUrl() {
    std::shared_lock lock(g_state.lua.mutex);
    return g_state.lua.eticketUrl;
}

// ============================ Mutations =====================================

void MarkOwned(steam::AppId appId) {
    std::unique_lock lock(g_state.lua.mutex);
    g_state.lua.familySharedAppIds.erase(appId);
    g_state.lua.ownedAppIds.insert(appId);
}

void MarkFamilyShared(steam::AppId appId) {
    std::unique_lock lock(g_state.lua.mutex);
    g_state.lua.ownedAppIds.erase(appId);
    g_state.lua.familySharedAppIds.insert(appId);
}

void BeginFile(const std::string& path) {
    std::unique_lock lock(g_state.lua.mutex);
    g_state.lua.currentFile = path;
}

void EndFile() {
    std::unique_lock lock(g_state.lua.mutex);
    g_state.lua.currentFile.clear();
}

void RecordDepot(steam::AppId depotId, const std::string& hexKey) {
    std::unique_lock lock(g_state.lua.mutex);
    auto& lua = g_state.lua;

    auto it = lua.depotKeys.find(depotId);
    if (it == lua.depotKeys.end()) {
        lua.depotKeys.emplace(depotId, hexKey);
    } else if (!hexKey.empty()) {
        it->second = hexKey;
    }

    lua.ownedAppIds.erase(depotId);
    lua.familySharedAppIds.erase(depotId);

    if (!lua.currentFile.empty()) {
        F& c = lua.fileContributions[lua.currentFile];
        if (c.depots.insert(depotId).second) {
            if (++lua.depotRefCount[depotId] == 1) {
                lua.pendingAdditions.push_back(depotId);
            }
        }
    }
}

void RecordLibraryApp(steam::AppId appId) {
    std::unique_lock lock(g_state.lua.mutex);
    auto& lua = g_state.lua;
    if (lua.currentFile.empty()) return;
    F& c = lua.fileContributions[lua.currentFile];
    if (c.libraryApps.insert(appId).second) {
        if (++lua.libraryRefCount[appId] == 1) {
            lua.libraryAppIds.insert(appId);
        }
    }
}

void SetAccessToken(steam::AppId appId, std::uint64_t token) {
    std::unique_lock lock(g_state.lua.mutex);
    auto& lua = g_state.lua;
    lua.accessTokens[appId] = token;
    if (!lua.currentFile.empty()) {
        lua.fileContributions[lua.currentFile].accessTokens[appId] = token;
    }
}

void SetManifestOverride(std::uint64_t depotId, ManifestOverride ov) {
    std::unique_lock lock(g_state.lua.mutex);
    auto& lua = g_state.lua;
    lua.manifestOverrides[depotId] = ov;
    if (!lua.currentFile.empty()) {
        lua.fileContributions[lua.currentFile].manifestOverrides[depotId] = ov;
    }
}

void SetEticketUrl(const std::string& url) {
    std::unique_lock lock(g_state.lua.mutex);
    auto& lua = g_state.lua;
    lua.eticketUrl = url;
    if (!lua.currentFile.empty()) {
        lua.fileContributions[lua.currentFile].eticketUrl = url;
    }
}

void UnloadFile(const std::string& path) {
    std::unique_lock lock(g_state.lua.mutex);
    auto& lua = g_state.lua;

    auto it = lua.fileContributions.find(path);
    if (it == lua.fileContributions.end()) return;

    F& c = it->second;

    // Ref-counted fields: remove depots and library apps whose count drops to 0.
    for (steam::AppId id : c.depots)
        DecRefAndCleanup(lua.depotKeys, lua.depotRefCount, id, lua.pendingRemovals);
    for (steam::AppId id : c.libraryApps)
        DecRefAndCleanup(lua.libraryAppIds, lua.libraryRefCount, id, lua.pendingRemovals);

    // Also clean owned/shared flags.
    for (steam::AppId id : c.depots) {
        lua.ownedAppIds.erase(id);
        lua.familySharedAppIds.erase(id);
    }

    std::size_t depotCount = c.depots.size();
    std::size_t libCount = c.libraryApps.size();
    std::size_t tokenCount = c.accessTokens.size();
    std::size_t manifestCount = c.manifestOverrides.size();
    bool eticketUrlChanged = !c.eticketUrl.empty();

    lua.fileContributions.erase(it);

    // Rebuild scalar fields from remaining files.
    RebuildScalars();

    if (depotCount || libCount || tokenCount || manifestCount || eticketUrlChanged) {
        AC_LOG_INFO(kModule,
                    "Unloaded %zu depot(s), %zu library app(s), %zu token(s), %zu manifest override(s), eticketUrlChanged=%d from %s.",
                    depotCount, libCount, tokenCount, manifestCount,
                    eticketUrlChanged ? 1 : 0, path.c_str());
    }
}

std::vector<steam::AppId> TakePendingAdditions() {
    std::unique_lock lock(g_state.lua.mutex);
    std::vector<steam::AppId> out;
    out.swap(g_state.lua.pendingAdditions);
    return out;
}

std::vector<steam::AppId> TakePendingRemovals() {
    std::unique_lock lock(g_state.lua.mutex);
    std::vector<steam::AppId> out;
    out.swap(g_state.lua.pendingRemovals);
    return out;
}

void ClearPendingChanges() {
    std::unique_lock lock(g_state.lua.mutex);
    g_state.lua.pendingAdditions.clear();
    g_state.lua.pendingRemovals.clear();
}

}  // namespace ac::luadata
