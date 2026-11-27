#pragma once

#include <cstddef>
#include <cstdint>
#include <vector>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Package-0 license injection, no-restart hot-reload, and update checks.
//
// `CPackageInfo`, `PackageInfo::appIdVec`/`depotIdVec`, `CUser` license state,
// and `CUserAppManager::CheckAppForUpdate` in steamclient64.dll have no
// internal locks and run on Steam's `CSteamEngine::BMainLoop` thread.
//
// Therefore NO worker or SteamUI thread ever mutates package 0 or calls into
// steamclient's license/update helpers directly: callers enqueue work into
// `g_state.licenseQueue`, and `TryDrainOnSteamThread()` executes it exclusively
// on the learned `BMainLoop` thread after the outermost license/package hook
// returns (`depth == 0`).
// ---------------------------------------------------------------------------
namespace ac::hooks::LicenseManager {

// Resolves CUtlMemoryGrow from steamclient and arms the package-0 monitor.
void Init(HMODULE steamclient);

// Stops and joins the startup monitor. Safe when Init never ran.
void Shutdown();

// Records the OS thread id of `CSteamEngine::BMainLoop` from hooks native to
// that loop (ProcessPendingLicenseUpdates, MarkLicenseAsChanged, RecvPkt,
// LoadPackage). Logged once per process.
void RecordSteamMainThread(const char* source);

// RAII nesting guard held around Steam license/package detours on any thread
// so TryDrainOnSteamThread() never mutates CPackageInfo/CUser while an outer
// BCheckUserLicensesUpdate / GetPackageInfo / LoadPackage is still walking it.
class SteamLicenseCallScope {
  public:
    SteamLicenseCallScope() noexcept;
    ~SteamLicenseCallScope() noexcept;
    SteamLicenseCallScope(const SteamLicenseCallScope&) = delete;
    SteamLicenseCallScope& operator=(const SteamLicenseCallScope&) = delete;
};

// Drains pending package-0 mutations, license refreshes, and update checks
// when running on `BMainLoop`, outside nested license/package calls and not
// already draining. Safe on any thread (fast atomic no-op otherwise).
void TryDrainOnSteamThread();

// Queues initial package-0 acquisition (or seeding once LoadPackage captured
// the pointer) to run on `BMainLoop`.
void RequestAcquirePackage0();
void RequestSeedPackage0(steam::PackageInfo* package0);

// Cached package-0 app-id vector size (lock-free, safe for StatusWriter).
std::size_t Package0AppCount();

// Queues an app for package-0 addition and license refresh on `BMainLoop`.
void RequestAppInPackage0(steam::AppId appId);

// Queues root app-id removal from package 0 when an app is confirmed as a
// real Valve family share after package 0 was already seeded.
void RequestRemoveAppFromPackage0(steam::AppId appId);

// Queues a license refresh without adding an app (e.g. family lender updates).
void RequestLicenseRefresh();

// Queues a per-app Steam client update check (`CheckAppForUpdate`) on `BMainLoop`.
void RequestUpdateCheck(steam::AppId appId);

// True while a SteamUI overview recheck should wait briefly so `BMainLoop`
// applies pending package-0 / hot-reload license changes first.
bool ShouldDeferUiRecheck(std::uint64_t nowMs);

// Called when CUser is first captured. Queues a deferred license refresh if
// package 0 was already seeded while CUser was null, and wakes the monitor.
void OnUserCaptured();

// Called after Steam's own ProcessPendingLicenseUpdates returns true so any
// previously unapplied Aether mutation triggers a post-rebuild UI recheck.
void OnSteamLicenseUpdatesApplied();

// Wakes the startup monitor immediately when CPackageInfo/CUser is captured.
void WakeStartupRetry();

// Hot-reload: queues pending Lua additions/removals and license refresh for
// `BMainLoop`. Safe from DirWatch and ManifestRestore worker threads.
void NotifyLicenseChanged();

} // namespace ac::hooks::LicenseManager
