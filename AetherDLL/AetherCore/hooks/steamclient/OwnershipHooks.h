#pragma once

#include "framework.h"

namespace ac::hooks {

// Registers the ownership-related hooks on the diverted steamclient
// (CheckAppOwnership, LoadPackage, GetPackageInfo, MarkLicenseAsChanged,
// SendCallbackToPipe) and resolves the CUtlMemory grow helper.
void RegisterOwnershipHooks(HMODULE diversion);

// Stops and joins the unlock-summary debounce thread. Safe to call when the
// hooks were never installed. Called from dllmain::Shutdown.
void ShutdownOwnershipHooks();

}  // namespace ac::hooks
