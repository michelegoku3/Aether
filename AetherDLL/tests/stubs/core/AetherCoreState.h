#pragma once
#include "core/Settings.h"
#include "hooks/wire/DonorPool.h"
#include <atomic>
#include <string>
namespace ac {
// Test seam: compile the REAL Settings.cpp without unrelated Steam/Win32 state.
// Path fields drive the wire-contract tests (manifest backup/restore
// fixtures); the hook-registry tests never touch them.
// The achievements member mirrors AetherCoreState::achievements exactly
// (same DonorPool::SharedState type), so DonorPool tests exercise the real
// ownership.
struct TestAchievements {
    hooks::DonorPool::SharedState donor;
};
struct TestCoreState {
    std::atomic<std::shared_ptr<const Settings>> settings{std::make_shared<const Settings>()};
    std::string steamInstallPath;
    std::string luaDir;
    std::string deskDataDir;
    TestAchievements achievements;
};
extern TestCoreState g_state;
} // namespace ac
