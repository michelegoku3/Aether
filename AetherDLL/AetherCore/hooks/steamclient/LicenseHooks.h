#pragma once

#include "framework.h"

namespace ac::hooks {

// Registers the license/controller/cloud compatibility hooks on steamclient:
//   * OptedInMask               — redirect controller opt-in queries from 480 to real app id.
//   * RequiresLegacyCDKey       — suppress legacy CD-key prompt for tracked apps.
//   * IsCloudEnabledForApp      — block Steam Cloud for managed-unowned apps, protecting local saves.
//   * GetRemoteStorageSyncState — return Synchronized for blocked apps, avoiding the cloud wait dialog.
//   * CloseAppCloud             — immediately close cloud session for blocked apps, preventing shutdown wait.
// All are optional: a missing pattern is recorded as a miss, not fatal.
void RegisterLicenseHooks(HMODULE diversion);

// Stops and joins the legacy-CD-key summary debounce thread. Safe to call
// when the hooks were never installed. Called from dllmain::Shutdown.
void ShutdownLicenseHooks();

} // namespace ac::hooks
