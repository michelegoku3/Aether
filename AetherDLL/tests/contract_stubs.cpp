#include "pch.h"

#include "core/AetherCoreState.h"

// Test-seam state (tests/stubs/core/AetherCoreState.h): paths + settings are
// assigned per fixture. The hook/wire TUs under test read only these.
namespace ac {
TestCoreState g_state;
} // namespace ac

// Out-of-process collaborators: never exercised by the contract tests, only
// linked. Any call reaching them is a test bug, hence the trap.
namespace ac::hooks::LicenseManager {
void NotifyLicenseChanged() {}
} // namespace ac::hooks::LicenseManager

namespace ac::script {
void ParseFile(const std::string&) {}
} // namespace ac::script
