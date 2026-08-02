#pragma once

#include <vector>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Package-0 license injection and no-restart hot-reload.
//
// Owns the single source of truth for mutating package 0's app-id vector and
// for asking Steam to re-evaluate licenses. Three entry points feed it:
//   * LoadPackage hook  -> SeedPackage0 (initial injection at package load)
//   * MarkLicenseAsChanged hook -> DoStartupInjection (post-login top-up)
//   * DirWatch          -> NotifyLicenseChanged (add/remove without restart)
//
// LumaCore spread this across PackagePatch + RuntimeCapture; here it is one
// module. The captured Steam pointers (CUser, CPackageInfo, package 0) live in
// AetherCoreState as write-once atomics — single source of truth, no copies;
// only the package-vector mutations are serialised by a module-local mutex.
// ---------------------------------------------------------------------------
namespace ac::hooks::LicenseManager {

// Resolves CUtlMemoryGrow, ProcessPendingLicenseUpdates and MarkLicenseAsChanged
// from the diversion, then starts the package-0 startup retry thread.
void Init(HMODULE diversion);

// Stops and joins the startup retry thread. Safe to call when Init never ran or
// the thread never started. Called from dllmain::Shutdown.
void Shutdown();

// Called from the LoadPackage hook when package 0 becomes available. Publishes
// the package-0 pointer to g_state and injects every configured depot exactly
// once (guarded by package0Seeded).
void SeedPackage0(steam::PackageInfo* package0);

// Called once post-login (MarkLicenseAsChanged). Tops up package 0 with any
// depots that were parsed after LoadPackage ran (startup race).
void DoStartupInjection();

// Hot-reload: applies pending additions/removals to package 0 and asks Steam to
// re-evaluate licenses. No UI card eviction (that path crashed Steam in
// LumaCore and is out of scope here).
void NotifyLicenseChanged();

}  // namespace ac::hooks::LicenseManager
