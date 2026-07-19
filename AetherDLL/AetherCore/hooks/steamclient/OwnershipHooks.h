#pragma once

#include "framework.h"

namespace ac::hooks {

// Registers the ownership-related hooks on the diverted steamclient
// (CheckAppOwnership, LoadPackage, GetPackageInfo, MarkLicenseAsChanged,
// SendCallbackToPipe) and resolves the CUtlMemory grow helper.
void RegisterOwnershipHooks(HMODULE diversion);

}  // namespace ac::hooks
