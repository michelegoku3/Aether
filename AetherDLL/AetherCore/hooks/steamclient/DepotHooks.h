#pragma once

#include "framework.h"

namespace ac::hooks {

// Registers depot hooks on the diverted steamclient: LoadDepotDecryptionKey
// (inject configured keys) and BuildDepotDependency (apply manifest overrides).
void RegisterDepotHooks(HMODULE diversion);

}  // namespace ac::hooks
