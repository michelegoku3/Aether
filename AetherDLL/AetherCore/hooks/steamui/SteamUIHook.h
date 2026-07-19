#pragma once

#include "framework.h"

namespace ac::hooks {

// Waits for steamui.dll (if not already loaded), then installs the
// LoadModuleWithPath redirect plus all steamclient hooks in one atomic batch,
// and finally publishes the hook status. Intended to run on the init thread or
// a dedicated worker.
void InstallAllHooks();

}  // namespace ac::hooks
