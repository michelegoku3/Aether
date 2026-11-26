#pragma once

#include <algorithm>
#include <cstdint>
#include <vector>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Single-thread dispatch policy for package-0 and Steam license updates.
//
// `CPackageInfo`, `PackageInfo::appIdVec`/`depotIdVec`, and `CUser`'s pending
// license vectors in steamclient64.dll have no internal locks: mutating or
// calling into them (`GetPackageInfo`, `CUtlMemoryGrow`, `MarkLicenseAsChanged`,
// `ProcessPendingLicenseUpdates`, `CheckAppForUpdate`) from worker threads or
// the SteamUI thread races `CSteamEngine::BMainLoop` (`CUser::RunFrame`) and
// corrupts the CPackageInfo RB-tree.
//
// Workers and UI hooks therefore only enqueue requests here; `BMainLoop` drains
// them once the outermost license/package call returns (`nestedSteamCallDepth
// == 0 && !alreadyDraining`). Pure header so the core unit suite pins every
// invariant without Steam.
// ---------------------------------------------------------------------------
namespace ac::hooks::license::policy {

constexpr bool CanDrainLicenseWork(bool onSteamMainThread, int nestedSteamCallDepth, bool alreadyDraining) noexcept {
    return onSteamMainThread && nestedSteamCallDepth == 0 && !alreadyDraining;
}

struct LicenseDispatchBatch {
    bool acquirePackage0 = false;
    bool hotReload = false;
    bool refresh = false;
    std::vector<steam::AppId> apps;
    std::vector<steam::AppId> updateCheckApps;

    [[nodiscard]] bool Empty() const noexcept {
        return !acquirePackage0 && !hotReload && !refresh && apps.empty() && updateCheckApps.empty();
    }
};

struct LicenseDispatchQueue {
    bool acquirePackage0 = false;
    bool hotReload = false;
    bool refresh = false;
    std::vector<steam::AppId> apps;
    std::vector<steam::AppId> updateCheckApps;
    std::uint64_t deferStartedMs = 0;

    void RequestAcquirePackage0() noexcept { acquirePackage0 = true; }
    void RequestHotReload() noexcept { hotReload = true; }
    void RequestRefresh() noexcept { refresh = true; }

    void RequestApp(steam::AppId appId) { AppendUnique(apps, appId); }
    void RequestUpdateCheck(steam::AppId appId) { AppendUnique(updateCheckApps, appId); }

    [[nodiscard]] bool HasPendingWork() const noexcept {
        return acquirePackage0 || hotReload || refresh || !apps.empty() || !updateCheckApps.empty();
    }

    // State changes that alter package 0 or trigger a Steam license evaluation
    // must settle before SteamUI rebuilds an overview card for the same reload.
    // Standalone `updateCheckApps` follow that rebuild and never block it.
    [[nodiscard]] bool HasPendingLicenseState() const noexcept {
        return acquirePackage0 || hotReload || refresh || !apps.empty();
    }

    LicenseDispatchBatch TakeBatch() {
        LicenseDispatchBatch out;
        out.acquirePackage0 = acquirePackage0;
        out.hotReload = hotReload;
        out.refresh = refresh;
        out.apps = std::move(apps);
        out.updateCheckApps = std::move(updateCheckApps);

        acquirePackage0 = false;
        hotReload = false;
        refresh = false;
        apps.clear();
        updateCheckApps.clear();
        deferStartedMs = 0;
        return out;
    }

    // Keeps SteamUI rechecks waiting briefly while BMainLoop drains package-0
    // or license state updates, bounded by `maxWaitMs` so an idle/offline
    // client never stalls the UI indefinitely.
    bool ShouldDeferUiRecheck(std::uint64_t nowMs, std::uint64_t maxWaitMs) noexcept {
        if (!HasPendingLicenseState()) {
            deferStartedMs = 0;
            return false;
        }
        if (deferStartedMs == 0) {
            deferStartedMs = nowMs;
            return maxWaitMs > 0;
        }
        return nowMs >= deferStartedMs && (nowMs - deferStartedMs) < maxWaitMs;
    }

  private:
    static void AppendUnique(std::vector<steam::AppId>& vec, steam::AppId appId) {
        if (appId == 0)
            return;
        if (std::find(vec.begin(), vec.end(), appId) == vec.end())
            vec.push_back(appId);
    }
};

} // namespace ac::hooks::license::policy
