#pragma once

#include "framework.h"

namespace ac::hooks {

// Registers the ConfigStoreGetBinary hook. This supplements DepotHooks:
// LoadDepotDecryptionKey covers one Steam path, ConfigStoreGetBinary covers the
// user-local config-store path and passively caches AppTickets Steam reads.
void RegisterDecryptionKeyHook(HMODULE diversion);

}  // namespace ac::hooks
