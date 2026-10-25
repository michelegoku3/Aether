#include "pch.h"
#include "hooks/license/LicenseManager.h"

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <limits>
#include <mutex>
#include <string>
#include <unordered_set>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "core/StructGuard.h"
#include "core/Workers.h"
#include "hooks/steamclient/OwnershipHooks.h"
#include "scripting/LuaData.h"
#include "utils/PatternEngine.h"

namespace ac::hooks::LicenseManager {
namespace {

constexpr const char* kModule = "License";
using steam::AppId;
using steam::PackageInfo;

// steamclient!CUtlMemory grow helper used to extend package 0's app-id vector.
using CUtlMemoryGrow_t = void (*)(steam::CUtlVector<AppId>*, int);
// steamclient!CClientAppManager::ProcessPendingLicenseUpdates(pCUser).
using ProcessPendingLicenseUpdates_t = bool (*)(void*);
// steamclient!CClientUser::MarkLicenseAsChanged(pCUser, packageId, reloadAll).
using MarkLicenseAsChanged_t = std::int64_t (*)(void*, std::uint32_t, bool);

CUtlMemoryGrow_t o_CUtlMemoryGrow = nullptr;
ProcessPendingLicenseUpdates_t o_ProcessPendingLicenseUpdates = nullptr;
MarkLicenseAsChanged_t o_MarkLicenseAsChanged = nullptr;

// The captured Steam object pointers (CUser, CPackageInfo, PackageInfo* of
// package 0) live in AetherCoreState as write-once atomics — single source of
// truth, read here without locking. s_packageMutationMutex only serialises package-vector
// mutations (LoadPackage hook thread, login thread, watcher thread can all
// reach here); it is module plumbing, not shared state.
std::mutex s_packageMutationMutex;

struct PackageContainmentResult {
    std::size_t expected = 0;
    std::size_t presentBefore = 0;
    std::size_t missingBefore = 0;
    std::size_t duplicatesRemoved = 0;
    std::size_t appended = 0;
    std::size_t presentAfter = 0;
    std::size_t missingAfter = 0;
    std::uint32_t total = 0;
    bool complete = false;
};

bool RemoveAllFromPackage0(PackageInfo* pkg, const std::vector<AppId>& ids, std::size_t& removed) {
    if (!pkg || !pkg->appIdVec.mem.memory || ids.empty())
        return true;
    std::unordered_set<AppId> removeSet;
    removeSet.reserve(ids.size());
    for (AppId id : ids) {
        if (id != 0)
            removeSet.insert(id);
    }

    AppId* data = pkg->appIdVec.mem.memory;
    std::uint32_t write = 0;
    for (std::uint32_t read = 0; read < pkg->appIdVec.size; ++read) {
        if (removeSet.count(data[read])) {
            ++removed;
            continue;
        }
        if (write != read)
            data[write] = data[read];
        ++write;
    }
    pkg->appIdVec.size = write;
    return true;
}

// Ensures each currently managed AppID occurs exactly once in package 0.
// Caller holds s_packageMutationMutex. Unmanaged Steam entries are preserved.
// `verbose` forces the INFO summary even when nothing changed; on-demand calls
// only log an actual append or duplicate removal.
PackageContainmentResult EnsurePackageContains(PackageInfo* pkg, const std::vector<AppId>& ids, const char* reason,
                                               bool verbose = true) {
    PackageContainmentResult out{};
    out.total = pkg ? pkg->appIdVec.size : 0;
    if (!pkg || !o_CUtlMemoryGrow) {
        AC_LOG_WARN(kModule, "Package0Containment reason=%s package/capacity unavailable.",
                    reason ? reason : "unknown");
        return out;
    }

    std::vector<AppId> expectedIds;
    std::unordered_set<AppId> expected;
    expected.reserve(ids.size());
    for (AppId id : ids) {
        if (id != 0 && expected.insert(id).second)
            expectedIds.push_back(id);
    }
    out.expected = expectedIds.size();

    // Struct guard before any pointer arithmetic on Valve's vector:
    // writing past allocationCount is heap corruption that surfaces
    // minutes later, somewhere else, with no usable evidence. The
    // invariants (size <= alloc, storage present and aligned, counts
    // not absurd) are checked on the live object, so a layout that
    // moved is caught here instead of in a crash dump.
    if (const auto guardReason = abi::guard::CheckPackage(pkg); !abi::guard::Passed(guardReason)) {
        abi::guard::CountRejection("PackageInfo (package 0)", guardReason);
        return out;
    }

    if (pkg->appIdVec.size > 0 && !pkg->appIdVec.mem.memory) {
        AC_LOG_ERROR(kModule, "Package0Containment reason=%s has null vector memory.", reason ? reason : "unknown");
        return out;
    }
    if (pkg->appIdVec.size > pkg->appIdVec.mem.allocationCount) {
        AC_LOG_ERROR(kModule, "Package0Containment reason=%s size=%u exceeds capacity=%u.", reason ? reason : "unknown",
                     pkg->appIdVec.size, pkg->appIdVec.mem.allocationCount);
        return out;
    }

    std::unordered_set<AppId> seen;
    seen.reserve(expected.size());
    AppId* data = pkg->appIdVec.mem.memory;
    std::uint32_t write = 0;
    for (std::uint32_t read = 0; read < pkg->appIdVec.size; ++read) {
        const AppId id = data[read];
        if (expected.count(id)) {
            if (!seen.insert(id).second) {
                ++out.duplicatesRemoved;
                continue;
            }
            ++out.presentBefore;
        }
        if (write != read)
            data[write] = id;
        ++write;
    }
    pkg->appIdVec.size = write;

    out.total = pkg->appIdVec.size;
    std::vector<AppId> missing;
    missing.reserve(expectedIds.size() - out.presentBefore);
    for (AppId id : expectedIds) {
        if (!seen.count(id))
            missing.push_back(id);
    }
    out.missingBefore = missing.size();
    out.presentAfter = out.presentBefore;

    if (!missing.empty()) {
        if (missing.size() > static_cast<std::size_t>(std::numeric_limits<int>::max())) {
            AC_LOG_ERROR(kModule, "Package0Containment reason=%s missing list too large.", reason ? reason : "unknown");
            return out;
        }
        const std::uint32_t oldSize = pkg->appIdVec.size;
        const std::size_t required = static_cast<std::size_t>(oldSize) + missing.size();
        if (required > std::numeric_limits<std::uint32_t>::max()) {
            AC_LOG_ERROR(kModule, "Package0Containment reason=%s size overflow.", reason ? reason : "unknown");
            return out;
        }
        o_CUtlMemoryGrow(&pkg->appIdVec, static_cast<int>(missing.size()));
        if (const auto grown = abi::guard::CheckPackage(pkg); !abi::guard::Passed(grown)) {
            // CUtlMemoryGrow left the vector in a shape we do not
            // recognise: stop before the append loop, not after it.
            abi::guard::CountRejection("PackageInfo after CUtlMemoryGrow", grown);
            return out;
        }
        if (!pkg->appIdVec.mem.memory || pkg->appIdVec.mem.allocationCount < required) {
            AC_LOG_ERROR(kModule, "CUtlMemoryGrow failed for package 0 (reason=%s).", reason ? reason : "unknown");
            return out;
        }
        for (std::size_t i = 0; i < missing.size(); ++i) {
            pkg->appIdVec.mem.memory[oldSize + i] = missing[i];
        }
        pkg->appIdVec.size = static_cast<std::uint32_t>(required);
        out.appended = missing.size();
        out.presentAfter += out.appended;
    }

    out.missingAfter = out.expected - out.presentAfter;
    out.total = pkg->appIdVec.size;
    out.complete = out.missingAfter == 0;
    const bool changed = out.appended != 0 || out.duplicatesRemoved != 0;
    if (verbose || changed) {
        AC_LOG_INFO(kModule,
                    "Package0Containment reason=%s expected=%zu present_before=%zu missing_before=%zu "
                    "duplicates_removed=%zu appended=%zu present_after=%zu missing_after=%zu total=%u complete=%d.",
                    reason ? reason : "unknown", out.expected, out.presentBefore, out.missingBefore,
                    out.duplicatesRemoved, out.appended, out.presentAfter, out.missingAfter, out.total,
                    out.complete ? 1 : 0);
    }
    return out;
}

bool Ready() {
    return g_state.pCUser.load() && g_state.pCPackageInfo.load() && g_state.pPackage0.load() && o_CUtlMemoryGrow &&
           o_ProcessPendingLicenseUpdates && o_MarkLicenseAsChanged;
}

// Asks Steam to re-evaluate licenses. Must run outside s_packageMutationMutex:
// these calls can synchronously re-enter hooks. Only the license captures are
// required; the package pointer is not (the app was already added).
void RefreshLicenses(const char* reason) {
    void* cUser = g_state.pCUser.load();
    auto markLicenseAsChanged = o_MarkLicenseAsChanged;
    auto processPendingLicenseUpdates = o_ProcessPendingLicenseUpdates;
    if (!cUser || !markLicenseAsChanged || !processPendingLicenseUpdates) {
        AC_LOG_DEBUG(kModule, "License refresh (%s) skipped: captures not ready.", reason ? reason : "unknown");
        return;
    }
    markLicenseAsChanged(cUser, 0, true);
    ++g_state.licenseReloadDirectCount;
    processPendingLicenseUpdates(cUser);
    AC_LOG_INFO(kModule, "License refresh (%s).", reason ? reason : "unknown");
    diag::Record("license_refresh", reason ? reason : "unknown");
}

// On-demand package-0 additions. A dedicated worker serialises the Steam
// vector mutation and the license refresh outside CheckAppOwnership, where
// Steam may be reading package 0 on the same thread.
std::mutex s_ondemandMutex;
std::condition_variable s_ondemandCv;
std::unordered_set<AppId> s_ondemandApps;
std::atomic<bool> s_ondemandStarted{false};
std::atomic<bool> s_refreshRequested{false};

void OnDemandWorker(std::atomic<bool>& stop) {
    for (;;) {
        std::vector<AppId> apps;
        bool refreshRequested = false;
        {
            std::unique_lock lock(s_ondemandMutex);
            s_ondemandCv.wait_for(lock, std::chrono::milliseconds(100), [&] {
                return stop.load(std::memory_order_relaxed) || !s_ondemandApps.empty() ||
                       s_refreshRequested.load(std::memory_order_relaxed);
            });
            if (stop.load(std::memory_order_relaxed))
                return;
            apps.assign(s_ondemandApps.begin(), s_ondemandApps.end());
            s_ondemandApps.clear();
            refreshRequested = s_refreshRequested.exchange(false);
        }
        if (apps.empty() && !refreshRequested)
            continue;
        bool changed = refreshRequested;
        for (AppId app : apps)
            changed = AddAppToPackage0(app) || changed;
        if (changed)
            RefreshLicenses("ondemand");
    }
}

void StartOnDemandWorker() {
    bool expected = false;
    if (!s_ondemandStarted.compare_exchange_strong(expected, true))
        return;
    if (!workers::StartWorker("license_ondemand", OnDemandWorker)) {
        s_ondemandStarted.store(false, std::memory_order_relaxed);
        AC_LOG_ERROR(kModule, "Could not start the on-demand package-0 worker; "
                              "apps will spoof but package 0 will not be topped up.");
    }
}

// Drops requests that predate a publish-time top-up: the top-up covers every
// app classified so far, and a new request after the clear is queued again.
void ClearPendingApps() {
    std::lock_guard<std::mutex> lock(s_ondemandMutex);
    s_ondemandApps.clear();
}

// -------------------------------------------------------------------
// Package-0 acquisition retry
//
// The one-shot LoadPackage(package 0) can be missed (late hook install,
// offline startup). A dedicated thread re-acquires the pointer on a
// throttled cadence until it is published or the budget is spent. It does
// not seed: apps enter package 0 on demand, after Steam's first ownership
// answer.
//
// The thread runs in the workers::StartWorker infrastructure (I-A): the
// central registry does stop+join during workers::Shutdown() with
// explicit logs; only the "already started" guard remains here.
// -------------------------------------------------------------------
std::atomic<bool> s_retryStarted{false};

// Marks the startup-retry idle so a later LicenseManager::Init (e.g.
// re-run of the hook batch after late pattern arrival) can start a
// fresh retry with a new full budget.
void MarkRetryIdle() { s_retryStarted.store(false, std::memory_order_relaxed); }

void StartupRetryWorker(std::atomic<bool>& stop) {
    // The budget is time-based, not attempt-based: it must expire even
    // when package 0 never becomes ready (e.g. Steam never loads it),
    // otherwise the thread would spin forever waiting.
    for (int attempts = 0; attempts < constants::kPackageRetryMaxAttempts; ++attempts) {
        if (stop.load(std::memory_order_relaxed)) {
            MarkRetryIdle();
            return;
        }
        if (g_state.package0Published.load(std::memory_order_relaxed)) {
            MarkRetryIdle();
            return;
        }

        // If the one-shot LoadPackage(package 0) was missed entirely
        // (late hook installation, e.g. pattern downloads on a fresh
        // build), pPackage0 is never published and waiting would stall
        // forever. Actively re-acquire package 0 through the hooked
        // GetPackageInfo entry point instead of only waiting for events.
        if (!g_state.pPackage0.load(std::memory_order_relaxed)) {
            ac::hooks::TryAcquirePackage0();
        }

        if (g_state.package0Published.load(std::memory_order_relaxed)) {
            AC_LOG_INFO(kModule, "Package 0 published after %d retry attempt(s).", attempts + 1);
            MarkRetryIdle();
            return;
        }
        if (attempts == 0 || attempts % 30 == 0) {
            AC_LOG_INFO(kModule, "Startup retry #%d: waiting for package 0.", attempts + 1);
        }

        // Sleep the interval in 100 ms steps: shutdown is still noticed within
        // 100 ms, the ~1 s re-attempt cadence is unaffected, and idle wakeups
        // are 10x fewer over the ~60 s budget.
        constexpr int kTickMs = 100;
        for (int elapsed = 0; elapsed < constants::kPackageRetryIntervalMs && !stop.load(std::memory_order_relaxed);
             elapsed += kTickMs) {
            Sleep(kTickMs);
        }
    }
    AC_LOG_WARN(kModule,
                "Package 0 was not published after %d attempt(s); on-demand additions will retry once it appears.",
                constants::kPackageRetryMaxAttempts);
    MarkRetryIdle();
}

void StartStartupRetry() {
    bool expected = false;
    if (!s_retryStarted.compare_exchange_strong(expected, true))
        return;
    if (!workers::StartWorker("license_package0_retry", StartupRetryWorker)) {
        s_retryStarted.store(false, std::memory_order_relaxed);
        AC_LOG_ERROR(kModule, "Could not start the package-0 startup retry worker; "
                              "top-up relies on LoadPackage/MarkLicenseAsChanged.");
    }
}

void StopStartupRetry() {
    // I-A: the join happens inside workers::Shutdown() (called BEFORE
    // LicenseManager::Shutdown in dllmain), so only tracking the request
    // remains here. MarkRetryIdle allows a full-budget retry when Init
    // runs again.
    MarkRetryIdle();
    AC_LOG_DEBUG(kModule, "Startup retry stop requested; join handled by workers::Shutdown.");
}

} // namespace

void Init(HMODULE diversion) {
    if (void* a = pattern::ResolveAddress("CUtlMemoryGrow", "steamclient", diversion)) {
        o_CUtlMemoryGrow = reinterpret_cast<CUtlMemoryGrow_t>(a);
    }
    if (void* a = pattern::ResolveAddress("ProcessPendingLicenseUpdates", "steamclient", diversion)) {
        o_ProcessPendingLicenseUpdates = reinterpret_cast<ProcessPendingLicenseUpdates_t>(a);
    }
    if (void* a = pattern::ResolveAddress("MarkLicenseAsChanged", "steamclient", diversion)) {
        o_MarkLicenseAsChanged = reinterpret_cast<MarkLicenseAsChanged_t>(a);
    }
    AC_LOG_INFO(kModule, "Init (grow=%d ppl=%d mlc=%d).", o_CUtlMemoryGrow != nullptr,
                o_ProcessPendingLicenseUpdates != nullptr, o_MarkLicenseAsChanged != nullptr);

    // Cover the missed-window cases: offline startup, late hook install.
    // No-op after package 0 is published.
    StartStartupRetry();
    // Serialises on-demand package-0 additions and license refreshes away from
    // the Steam hook threads.
    StartOnDemandWorker();
}

void Shutdown() {
    StopStartupRetry();
    s_ondemandStarted.store(false, std::memory_order_relaxed);
}

void SeedPackage0(PackageInfo* package0) {
    bool changed = false;
    {
        std::lock_guard<std::mutex> lock(s_packageMutationMutex);
        // Single store point: SeedPackage0 owns the pPackage0 publication so
        // the pointer and the mode flag stay atomic w.r.t. other mutators.
        g_state.pPackage0.store(package0);
        g_state.package0Published.store(package0 != nullptr);
        // If Steam published package 0 after some apps were already
        // classified, top up only those classified apps. Unclassified apps
        // stay out, so their first ownership answer is still Steam's real one.
        // Requests queued before this point are covered by the top-up.
        if (package0) {
            ClearPendingApps();
            const std::vector<AppId> ids = luadata::Package0Ids();
            if (!ids.empty()) {
                const PackageContainmentResult result = EnsurePackageContains(package0, ids, "publish");
                changed = result.appended != 0 || result.duplicatesRemoved != 0;
            }
        }
    }
    // Refresh outside the mutation lock: the callbacks can re-enter hooks.
    if (changed)
        RefreshLicenses("publish");
    AC_LOG_INFO(kModule, "Package 0 published; licenses are added per app after Steam classification.");
}

bool AddAppToPackage0(AppId appId) {
    if (appId == 0)
        return false;

    // Package 0 may not have been published when the app was classified (the
    // startup retry can finish before login). Acquire it here, before taking
    // the mutation lock: TryAcquirePackage0 calls SeedPackage0, which locks it.
    if (!g_state.pPackage0.load(std::memory_order_acquire) && !ac::hooks::TryAcquirePackage0()) {
        AC_LOG_DEBUG_ONCE(kModule, "AddAppToPackage0 skipped: package 0 is not available yet.");
        return false;
    }

    std::lock_guard<std::mutex> lock(s_packageMutationMutex);
    auto* pkg = static_cast<PackageInfo*>(g_state.pPackage0.load());
    if (!pkg || !o_CUtlMemoryGrow) {
        AC_LOG_DEBUG_ONCE(kModule, "AddAppToPackage0 skipped: package0=%d grow=%d.", pkg != nullptr,
                          o_CUtlMemoryGrow != nullptr);
        return false;
    }

    const std::vector<AppId> ids = luadata::IdsForApp(appId);
    if (ids.empty()) {
        AC_LOG_DEBUG(kModule, "AddAppToPackage0(%u): no lua ids.", appId);
        return false;
    }
    const PackageContainmentResult result = EnsurePackageContains(pkg, ids, "ondemand", false);
    return result.appended != 0 || result.duplicatesRemoved != 0;
}

void RequestAppInPackage0(AppId appId) {
    if (appId == 0)
        return;
    {
        std::lock_guard<std::mutex> lock(s_ondemandMutex);
        s_ondemandApps.insert(appId);
    }
    s_ondemandCv.notify_one();
}

void RequestLicenseRefresh() {
    s_refreshRequested.store(true, std::memory_order_relaxed);
    s_ondemandCv.notify_one();
}

void NotifyLicenseChanged() {
    std::vector<AppId> removals;
    std::vector<AppId> additions;
    std::size_t removed = 0;
    std::uint32_t total = 0;
    void* cUser = nullptr;
    auto markLicenseAsChanged = o_MarkLicenseAsChanged;
    auto processPendingLicenseUpdates = o_ProcessPendingLicenseUpdates;

    {
        std::lock_guard<std::mutex> lock(s_packageMutationMutex);
        if (!Ready()) {
            AC_LOG_WARN(kModule, "NotifyLicenseChanged: captures not ready; skipping.");
            return;
        }

        removals = luadata::TakePendingRemovals();
        additions = luadata::TakePendingAdditions();
        if (removals.empty() && additions.empty()) {
            AC_LOG_DEBUG(kModule, "NotifyLicenseChanged: no pending changes; skipping.");
            return;
        }

        auto* pkg = static_cast<PackageInfo*>(g_state.pPackage0.load());
        if (!pkg) {
            AC_LOG_WARN(kModule, "NotifyLicenseChanged: package 0 unavailable; skipping.");
            return;
        }
        RemoveAllFromPackage0(pkg, removals, removed);
        // Reconcile against the classified set, not every configured id: an
        // app Steam has not classified yet must stay out of package 0 so its
        // first CheckAppOwnership sees the real owned/family answer. This also
        // repairs duplicates and makes reloads idempotent.
        const PackageContainmentResult result = EnsurePackageContains(pkg, luadata::Package0Ids(), "hotreload");
        if (!result.complete)
            AC_LOG_WARN(kModule, "Package 0 hot-reload containment incomplete; the next add repairs it.");

        total = pkg->appIdVec.size;
        cUser = g_state.pCUser.load();
        markLicenseAsChanged = o_MarkLicenseAsChanged;
        processPendingLicenseUpdates = o_ProcessPendingLicenseUpdates;
    }

    // Ask Steam to re-evaluate package state outside s_packageMutationMutex. These calls can
    // synchronously trigger callbacks/hooks; keeping our mutation lock held here
    // would make re-entrancy and deadlocks much more likely.
    markLicenseAsChanged(cUser, 0, true);
    ++g_state.licenseReloadDirectCount;
    processPendingLicenseUpdates(cUser);

    AC_LOG_INFO(kModule, "License refresh: +%zu -%zu (total %u).", additions.size(), removed, total);
    diag::Record("license_refresh", "+" + std::to_string(additions.size()) + " -" + std::to_string(removed));
}

} // namespace ac::hooks::LicenseManager
