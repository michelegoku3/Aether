#pragma once
#include "core/Settings.h"
#include <atomic>
namespace ac {
// Test seam: compile the REAL Settings.cpp without unrelated Steam/Win32 state.
struct TestCoreState {
    std::atomic<std::shared_ptr<const Settings>> settings{std::make_shared<const Settings>()};
};
extern TestCoreState g_state;
}
