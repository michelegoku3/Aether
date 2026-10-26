#pragma once
#include <atomic>
#include <cstdint>
#include <memory>
#include <shared_mutex>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include "core/Settings.h"
#include "core/SteamTypes.h"
#include "hooks/wire/DonorPool.h"
namespace ac {
// Test seam: compile the REAL Settings.cpp and LuaData.cpp without unrelated
// Steam/Win32 state. Path fields drive the wire-contract tests (manifest
// backup/restore fixtures); the hook-registry tests never touch them.
// The achievements member mirrors AetherCoreState::achievements exactly
// (same DonorPool::SharedState type), so DonorPool tests exercise the real
// ownership. LuaStore mirrors the production sub-struct field for field, so
// LuaData.cpp compiles here unchanged: a field added there breaks this header
// until the mirror is updated.
struct ManifestOverride {
    std::uint64_t gid = 0;
    std::uint64_t size = 0;
};
struct AetherCoreState {
    struct LuaStore {
        mutable std::shared_mutex mutex;
        std::unordered_map<steam::AppId, std::string> depotKeys;
        std::unordered_map<steam::AppId, std::uint64_t> accessTokens;
        std::unordered_map<std::uint64_t, ManifestOverride> manifestOverrides;
        std::unordered_set<steam::AppId> ownedAppIds;
        std::unordered_set<steam::AppId> familySharedAppIds;
        std::unordered_set<steam::AppId> spoofedAppIds;
        std::unordered_map<steam::AppId, std::uint32_t> familyLenderAppIds;
        std::unordered_set<steam::AppId> libraryAppIds;
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
        std::vector<steam::AppId> pendingAdditions;
        std::vector<steam::AppId> pendingRemovals;
        std::string currentFile;
    };
    LuaStore lua;
    std::atomic<std::shared_ptr<const Settings>> settings{std::make_shared<const Settings>()};
    std::atomic<void*> pCUtlMemoryGrow{nullptr};
    std::string steamInstallPath;
    std::string luaDir;
    std::string deskDataDir;
    struct TestAchievements {
        hooks::DonorPool::SharedState donor;
    };
    TestAchievements achievements;
};
// The translation units that own the definition spell the type by this name.
using TestCoreState = AetherCoreState;
extern AetherCoreState g_state;
} // namespace ac
