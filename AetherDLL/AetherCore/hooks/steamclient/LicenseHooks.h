#pragma once

#include "framework.h"

namespace ac::hooks {

	// Registers the license/controller/cloud compatibility hooks on steamclient:
	//   * OptedInMask                — redirect controller opt-in queries from 480 to real app id.
	//   * RequiresLegacyCDKey        — suppress legacy CD-key prompt for tracked apps.
	//   * IsCloudEnabledForApp       — block Steam Cloud for managed-unowned apps, protecting local saves.
	//   * EvaluateRemoteStorageSyncState — prevent AutoCloud evaluation for blocked apps (fixes cloud error popup).
	//   * RunAutoCloudOnAppLaunch    — prevent AutoCloud launch sync for blocked apps.
	//   * RunAutoCloudOnAppExit      — prevent AutoCloud exit sync for blocked apps (must return success to avoid shutdown hang).
	//   * GetRemoteStorageSyncState  — return Disabled (0) for blocked apps, fixing infinite "Finishing cloud sync" dialog.
	//   * CloseAppCloud              — immediately close cloud session for blocked apps, preventing shutdown wait.
	//   * SetCloudEnabledForApp      — ignore attempts to re-enable cloud for blocked apps.
	// All are optional: a missing pattern is recorded as a miss, not fatal.
	void RegisterLicenseHooks(HMODULE diversion);

	// Stops and joins the legacy-CD-key summary debounce thread. Safe to call
	// when the hooks were never installed. Called from dllmain::Shutdown.
	void ShutdownLicenseHooks();

}  // namespace ac::hooks
