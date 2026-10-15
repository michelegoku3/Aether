#pragma once
#include "core/Settings.h"
#include <atomic>
#include <string>
namespace ac {
// Test seam: compile the REAL Settings.cpp without unrelated Steam/Win32 state.
// Path fields drive the wire-contract tests (manifest backup/restore
// fixtures); the hook-registry tests never touch them.
struct TestCoreState {
    std::atomic<std::shared_ptr<const Settings>> settings{std::make_shared<const Settings>()};
    std::string steamInstallPath;
    std::string luaDir;
    std::string deskDataDir;
};
extern TestCoreState g_state;
} // namespace ac
