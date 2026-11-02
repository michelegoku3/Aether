#pragma once

#include <vector>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Package-0 license injection and no-restart hot-reload.
//
// Owns the single source of truth for mutating package 0's app-id vector and
// for asking Steam to re-evaluate licenses. Entry points:
//   * LoadPackage / GetPackageInfo -> SeedPackage0 (publish the pointer)
//   * CheckAppOwnership            -> RequestAppInPackage0 (after classification)
//   * DirWatch                     -> NotifyLicenseChanged (hot-reload)
//
// The captured Steam pointers (CUser, CPackageInfo, package 0) live in
// AetherCoreState as write-once atomics — single source of truth, no copies;
// only the package-vector mutations are serialised by a module-local mutex.
// ---------------------------------------------------------------------------
namespace ac::hooks::LicenseManager {

// Resolves CUtlMemoryGrow, ProcessPendingLicenseUpdates and MarkLicenseAsChanged
// from the steamclient, then starts the package-0 startup retry thread.
void Init(HMODULE steamclient);

// Stops and joins the startup retry thread. Safe to call when Init never ran or
// the thread never started. Called from dllmain::Shutdown.
void Shutdown();

// Called from the LoadPackage/GetPackageInfo hooks when package 0 becomes
// available. Publishes the package-0 pointer to g_state and seeds every
// configured .lua id into it (Steam lists an app only when a license mentions
// it). On-demand additions after classification are a top-up, not the seed.
void SeedPackage0(steam::PackageInfo* package0);

// Number of ids currently in package 0, read under the mutation lock so a
// concurrent grow cannot tear the vector. 0 when the package is not published.
std::size_t Package0AppCount();

// Adds the app and its .lua depots to package 0 synchronously. Returns true
// when the vector changed (an id was appended or a duplicate removed). Used by
// the publish-time top-up and by the on-demand worker; never call it from a
// Steam hook thread while Steam may be reading package 0.
bool AddAppToPackage0(steam::AppId appId);

// Called from CheckAppOwnership after Steam classified appId. Queues the app
// on the license worker, which adds it to package 0 and refreshes licenses
// outside the hook thread. Coalesces concurrent requests.
void RequestAppInPackage0(steam::AppId appId);

// Queues a license refresh without adding an app. Used after the family lender
// map changes an app's classification, so the UI re-reads its ownership.
void RequestLicenseRefresh();

// Hot-reload: applies pending additions/removals to package 0 and asks Steam to
// re-evaluate licenses. No UI card eviction (crash-prone; out of scope here).
void NotifyLicenseChanged();

} // namespace ac::hooks::LicenseManager
