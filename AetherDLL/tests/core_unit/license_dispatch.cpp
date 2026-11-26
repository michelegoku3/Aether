#include "cases.h"

#include <cstdint>
#include <vector>

#include "hooks/license/LicenseDispatchPolicy.h"

namespace {

using ac::hooks::license::policy::CanDrainLicenseWork;
using ac::hooks::license::policy::LicenseDispatchBatch;
using ac::hooks::license::policy::LicenseDispatchQueue;
using ac::steam::AppId;

void OnlyDrainsOnMainThreadOutermostUnnested() {
    CHECK(!CanDrainLicenseWork(false, 0, false));
    CHECK(!CanDrainLicenseWork(true, 1, false));
    CHECK(!CanDrainLicenseWork(true, 2, false));
    CHECK(!CanDrainLicenseWork(true, 0, true));
    CHECK(CanDrainLicenseWork(true, 0, false));
}

void CoalescesRequestsAndPreservesFirstSeenOrder() {
    LicenseDispatchQueue q;
    CHECK(!q.HasPendingWork());
    CHECK(!q.HasPendingLicenseState());

    q.RequestAcquirePackage0();
    q.RequestHotReload();
    q.RequestRefresh();
    q.RequestApp(0);
    q.RequestApp(3618390);
    q.RequestApp(4809710);
    q.RequestApp(3618390);
    q.RequestUpdateCheck(0);
    q.RequestUpdateCheck(3618390);
    q.RequestUpdateCheck(3618390);

    CHECK(q.HasPendingWork());
    CHECK(q.HasPendingLicenseState());

    const LicenseDispatchBatch batch = q.TakeBatch();
    CHECK(!batch.Empty());
    CHECK(batch.acquirePackage0);
    CHECK(batch.hotReload);
    CHECK(batch.refresh);
    CHECK(batch.apps == (std::vector<AppId>{3618390, 4809710}));
    CHECK(batch.updateCheckApps == (std::vector<AppId>{3618390}));

    // Taking the batch empties the pending queue until new work arrives.
    CHECK(!q.HasPendingWork());
    CHECK(!q.HasPendingLicenseState());
    CHECK(q.TakeBatch().Empty());
}

void UpdateChecksAloneDoNotDeferUiRecheck() {
    LicenseDispatchQueue q;
    q.RequestUpdateCheck(3618390);
    CHECK(q.HasPendingWork());
    CHECK(!q.HasPendingLicenseState());
    CHECK(!q.ShouldDeferUiRecheck(1000, 500));
}

void UiRecheckDefersWhileLicenseWorkPendingUpToTimeout() {
    LicenseDispatchQueue q;
    q.RequestHotReload();

    CHECK(q.ShouldDeferUiRecheck(1000, 500));
    CHECK(q.ShouldDeferUiRecheck(1499, 500));
    // Once the wait budget expires, UI rechecks stop waiting so an idle or
    // offline client cannot stall card updates indefinitely.
    CHECK(!q.ShouldDeferUiRecheck(1500, 500));

    // Draining clears the state and resets the defer timer for future batches.
    const auto batch = q.TakeBatch();
    CHECK(!batch.Empty());
    CHECK(!q.ShouldDeferUiRecheck(1600, 500));

    q.RequestApp(3618390);
    CHECK(q.ShouldDeferUiRecheck(2000, 500));
    CHECK(q.ShouldDeferUiRecheck(2400, 500));
    CHECK(!q.ShouldDeferUiRecheck(2500, 500));
}

} // namespace

void LicenseDispatchTest() {
    OnlyDrainsOnMainThreadOutermostUnnested();
    CoalescesRequestsAndPreservesFirstSeenOrder();
    UpdateChecksAloneDoNotDeferUiRecheck();
    UiRecheckDefersWhileLicenseWorkPendingUpToTimeout();
}
