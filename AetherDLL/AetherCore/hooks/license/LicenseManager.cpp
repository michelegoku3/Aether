#include "pch.h"
#include "hooks/license/LicenseManager.h"

#include <algorithm>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <mutex>
#include <string>
#include <unordered_set>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/FamilyClassification.h"
#include "core/Logger.h"
#include "core/Settings.h"
#include "core/StructGuard.h"
#include "core/Workers.h"
#include "credentials/CredentialStore.h"
#include "diagnostics/StatusWriter.h"
#include "family/FamilyLenders.h"
#include "hooks/license/LicenseDispatchPolicy.h"
#include "hooks/license/PackageSeedPolicy.h"
#include "hooks/steamclient/OwnershipHooks.h"
#include "hooks/steamui/AppOverviewHooks.h"
#include "scripting/LuaData.h"
#include "utils/PatternEngine.h"

namespace ac::hooks::LicenseManager {
namespace {

constexpr const char* kModule = "License";
constexpr std::uint64_t kMaxUiRecheckDeferMs = 500;
using steam::AppId;
using steam::PackageInfo;
namespace policy = ac::hooks::license::policy;

// Per-thread re-entrancy counters (Rule (c) in AetherCoreState.h:
// thread_local scratch state is never shared across threads).
thread_local int t_steamLicenseDepth = 0;
thread_local bool t_draining = false;

bool IsSteamMainThread() noexcept {
    const std::uint32_t expected = g_state.steamClientThreadId.load(std::memory_order_acquire);
    return expected != 0 && GetCurrentThreadId() == expected;
}

template <typename Fn> void EnqueueWork(Fn&& mutate) {
    {
        std::lock_guard<std::mutex> lock(g_state.licenseQueueMutex);
        mutate(g_state.licenseQueue);
        g_state.licenseWorkPending.store(g_state.licenseQueue.HasPendingWork(), std::memory_order_release);
    }
}

// A real Steam Family licence belongs to Valve: package 0 must not manufacture
// one. The client otherwise offers PLAY while the CM stops the session minutes
// later. Depots stay seeded - they are content, not a licence - and the opt-in
// owner banner keeps its app, because that stamp is Aether's own.
bool IsValveFamilyLicense(AppId appId) {
    if (luadata::IsRealFamilyShare(appId))
        return true;
    const std::uint32_t lender = ac::family::LenderFor(appId);
    if (lender == 0)
        return false;
    const std::uint32_t active = credential::ReadActiveUserId();
    const auto settings = Settings::Snapshot();
    if (!settings)
        return familyclass::IsValveFamilyLicense(appId, lender, active, false, {}, 0);
    return familyclass::IsValveFamilyLicense(appId, lender, active, settings->libraryOwnerBanner,
                                             settings->libraryOwnerBannerApps, settings->libraryOwnerBannerSteamId);
}

bool IsOwnBannerRoot(AppId appId) {
    if (luadata::IsFamilyShared(appId) && !luadata::IsRealFamilyShare(appId))
        return true;
    const std::uint32_t lender = ac::family::LenderFor(appId);
    if (lender == 0)
        return false;
    const auto settings = Settings::Snapshot();
    if (!settings)
        return false;
    return familyclass::IsOwnBannerStamp(appId, lender, settings->libraryOwnerBanner, settings->libraryOwnerBannerApps,
                                         settings->libraryOwnerBannerSteamId, credential::ReadActiveUserId());
}

// Collects any active Valve family-shared root ids so package-0 seeds and
// hot-reloads keep them out of both appIdVec and depotIdVec without dropping
// distinct companion DLC or depot ids.
std::vector<AppId> FilterFamilyIdsOut(const std::vector<AppId>& ids, const char* context) {
    std::vector<AppId> excluded;
    for (const AppId id : ids) {
        if (id != 0 && IsValveFamilyLicense(id))
            excluded.push_back(id);
    }
    if (excluded.empty())
        return policy::NormalizeSeed(ids);
    const std::vector<AppId> filtered = policy::ExcludeIds(ids, excluded);
    AC_LOG_INFO(kModule, "Package-0 %s: skipped %zu active family-shared root id(s) (%zu kept).",
                context ? context : "seed", ids.size() - filtered.size(), filtered.size());
    return filtered;
}

// The ids this session may seed into package 0: everything the .lua files
// configure, minus the apps Valve licenses to another account.
std::vector<AppId> SeedableIds() { return FilterFamilyIdsOut(luadata::SeedPackage0Ids(), "seed"); }

std::vector<AppId> ActiveFamilyRootIds() {
    std::vector<AppId> familyRoots;
    for (const AppId root : luadata::LibraryAppIds()) {
        if (root != 0 && IsValveFamilyLicense(root))
            familyRoots.push_back(root);
    }
    return familyRoots;
}

// steamclient!CUtlMemory grow helper used to extend package 0's app-id vector.
using CUtlMemoryGrow_t = void (*)(steam::CUtlVector<AppId>*, int);
CUtlMemoryGrow_t o_CUtlMemoryGrow = nullptr;

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

bool RemoveAllFromPackage0(PackageInfo* pkg, steam::CUtlVector<AppId>& vec, const std::vector<AppId>& ids,
                           std::size_t& removed) {
    if (!pkg || !vec.mem.memory || ids.empty())
        return true;
    const auto removeSet = policy::BuildNonZeroSet(ids);
    removed += policy::PruneInPlace(vec.mem.memory, vec.size, removeSet);
    return true;
}

// Ensures each currently managed AppID occurs exactly once in one of package 0's
// id vectors. Caller must run on steamclient's BMainLoop thread. Unmanaged Steam
// entries are preserved.
PackageContainmentResult EnsurePackageContains(PackageInfo* pkg, steam::CUtlVector<AppId>& vec,
                                               const std::vector<AppId>& ids, const char* reason, bool verbose = true) {
    PackageContainmentResult out{};
    out.total = vec.size;
    if (!pkg || !o_CUtlMemoryGrow) {
        AC_LOG_WARN(kModule, "Package0Containment reason=%s package/capacity unavailable.",
                    reason ? reason : "unknown");
        return out;
    }

    const std::vector<AppId> expectedIds = policy::NormalizeSeed(ids);
    const std::unordered_set<AppId> expected(expectedIds.begin(), expectedIds.end());
    out.expected = expectedIds.size();

    if (const auto guardReason = abi::guard::CheckPackage(pkg); !abi::guard::Passed(guardReason)) {
        abi::guard::CountRejection("PackageInfo (package 0)", guardReason);
        return out;
    }

    if (vec.size > 0 && !vec.mem.memory) {
        AC_LOG_ERROR(kModule, "Package0Containment reason=%s has null vector memory.", reason ? reason : "unknown");
        return out;
    }
    if (vec.size > vec.mem.allocationCount) {
        AC_LOG_ERROR(kModule, "Package0Containment reason=%s size=%u exceeds capacity=%u.", reason ? reason : "unknown",
                     vec.size, vec.mem.allocationCount);
        return out;
    }

    std::unordered_set<AppId> seen;
    seen.reserve(expected.size());
    AppId* data = vec.mem.memory;
    std::uint32_t write = 0;
    for (std::uint32_t read = 0; read < vec.size; ++read) {
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
    vec.size = write;

    out.total = vec.size;
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
        const std::uint32_t oldSize = vec.size;
        const std::size_t required = static_cast<std::size_t>(oldSize) + missing.size();
        if (required > std::numeric_limits<std::uint32_t>::max()) {
            AC_LOG_ERROR(kModule, "Package0Containment reason=%s size overflow.", reason ? reason : "unknown");
            return out;
        }
        o_CUtlMemoryGrow(&vec, static_cast<int>(missing.size()));
        if (const auto grown = abi::guard::CheckPackage(pkg); !abi::guard::Passed(grown)) {
            abi::guard::CountRejection("PackageInfo after CUtlMemoryGrow", grown);
            return out;
        }
        if (!vec.mem.memory || vec.mem.allocationCount < required) {
            AC_LOG_ERROR(kModule, "CUtlMemoryGrow failed for package 0 (reason=%s).", reason ? reason : "unknown");
            return out;
        }
        for (std::size_t i = 0; i < missing.size(); ++i) {
            vec.mem.memory[oldSize + i] = missing[i];
        }
        vec.size = static_cast<std::uint32_t>(required);
        out.appended = missing.size();
        out.presentAfter += out.appended;
    }

    out.missingAfter = out.expected - out.presentAfter;
    out.total = vec.size;
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

struct PackageSeedResult {
    PackageContainmentResult apps;
    PackageContainmentResult depots;

    bool Changed() const {
        return apps.appended != 0 || apps.duplicatesRemoved != 0 || depots.appended != 0 ||
               depots.duplicatesRemoved != 0;
    }
};

PackageSeedResult SeedPackageVectors(PackageInfo* pkg, const std::vector<AppId>& ids, const char* reason,
                                     bool verbose) {
    PackageSeedResult out{};
    if (!pkg)
        return out;
    const std::string depotReason = std::string(reason ? reason : "unknown") + "-depots";
    out.apps = EnsurePackageContains(pkg, pkg->appIdVec, ids, reason, verbose);
    out.depots = EnsurePackageContains(pkg, pkg->depotIdVec, ids, depotReason.c_str(), verbose);
    g_state.package0AppCount.store(static_cast<std::size_t>(pkg->appIdVec.size), std::memory_order_relaxed);
    return out;
}

// Asks Steam to re-evaluate licenses. Must run on `BMainLoop` (`t_draining ==
// true`) so Steam's `BCheckUserLicensesUpdate` and `CPackageInfo` RB-tree are
// never mutated concurrently by two threads.
bool RefreshLicensesOnSteamThread(const char* reason) {
    void* cUser = g_state.pCUser.load(std::memory_order_acquire);
    if (!cUser || !ac::hooks::HasLicenseRefreshHelpers()) {
        g_state.licenseRefreshPending.store(true, std::memory_order_release);
        AC_LOG_WARN(kModule, "License refresh (%s) deferred: CUser/captures not ready.", reason ? reason : "unknown");
        return false;
    }
    const bool wasDeferred = g_state.licenseRefreshPending.exchange(false, std::memory_order_acq_rel);
    ac::hooks::MarkLicenseAsChangedOnSteamThread(cUser, 0, true);
    ++g_state.licenseReloadDirectCount;
    const bool applied = ac::hooks::ProcessPendingLicenseUpdatesOnSteamThread(cUser);
    if (g_state.pPackage0.load(std::memory_order_acquire) != nullptr)
        g_state.package0Published.store(true, std::memory_order_release);
    AC_LOG_INFO(kModule, "License refresh (%s, applied=%d, tid=%lu).", reason ? reason : "unknown", applied ? 1 : 0,
                static_cast<unsigned long>(GetCurrentThreadId()));
    diag::Record("license_refresh", reason ? reason : "unknown");

    const bool steamUiStarted = g_state.overview.fills.load(std::memory_order_relaxed) > 0 ||
                                g_state.overview.appController.load(std::memory_order_acquire) != nullptr ||
                                g_state.overview.appChangeSource.load(std::memory_order_acquire) != nullptr;
    if (wasDeferred || steamUiStarted) {
        const std::vector<AppId> roots = luadata::LibraryAppIds();
        if (!roots.empty())
            hooks::steamui::RequestAppRecheck(roots);
    }
    return true;
}

bool AddAppToPackage0OnSteamThread(AppId appId) {
    if (appId == 0 || IsValveFamilyLicense(appId))
        return false;

    auto* pkg = static_cast<PackageInfo*>(g_state.pPackage0.load(std::memory_order_acquire));
    if (!pkg && g_state.pCPackageInfo.load(std::memory_order_acquire) != nullptr) {
        pkg = ac::hooks::AcquirePackage0OnSteamThread();
    }
    if (!pkg || !o_CUtlMemoryGrow) {
        AC_LOG_DEBUG_ONCE(kModule, "AddAppToPackage0 deferred: package0=%d grow=%d.", pkg != nullptr,
                          o_CUtlMemoryGrow != nullptr);
        return false;
    }

    const std::vector<AppId> ids = FilterFamilyIdsOut(luadata::IdsForApp(appId), "ondemand");
    if (ids.empty()) {
        AC_LOG_DEBUG(kModule, "AddAppToPackage0(%u): no non-family lua ids.", appId);
        return false;
    }
    const PackageSeedResult result = SeedPackageVectors(pkg, ids, "ondemand", false);
    return result.Changed();
}

void RemoveAppsFromPackage0OnSteamThread(const std::vector<AppId>& requestedRemovals, bool& didMutate,
                                         bool& needRefresh) {
    if (requestedRemovals.empty())
        return;

    std::vector<AppId> candidates;
    candidates.reserve(requestedRemovals.size());
    for (const AppId id : requestedRemovals) {
        if (id == 0)
            continue;
        const bool isValveFamily = IsValveFamilyLicense(id);
        const bool isSpoofed = luadata::IsSpoofed(id);
        const bool isOwnBanner = (!isValveFamily && !isSpoofed) ? IsOwnBannerRoot(id) : false;
        if (policy::ShouldPruneRequestedApp(isValveFamily, isSpoofed, isOwnBanner))
            candidates.push_back(id);
    }
    if (candidates.empty())
        return;

    auto* pkg = static_cast<PackageInfo*>(g_state.pPackage0.load(std::memory_order_acquire));
    if (!pkg)
        return;
    if (const auto guardReason = abi::guard::CheckPackage(pkg); !abi::guard::Passed(guardReason)) {
        abi::guard::CountRejection("PackageInfo (family prune)", guardReason);
        return;
    }

    // Strip the borrowed root app id from BOTH appIdVec and depotIdVec:
    // CUser::RebuildLicensesCache indexes appToLicenses from both vectors, so
    // leaving a root app id in depotIdVec re-attaches package 0 on the next
    // rebuild. Companion DLC/depot ids (!= root app id) stay intact in both.
    std::size_t removedApps = 0;
    std::size_t removedDepots = 0;
    RemoveAllFromPackage0(pkg, pkg->appIdVec, candidates, removedApps);
    RemoveAllFromPackage0(pkg, pkg->depotIdVec, candidates, removedDepots);
    if (removedApps == 0 && removedDepots == 0)
        return;

    const std::uint32_t total = pkg->appIdVec.size;
    g_state.package0AppCount.store(static_cast<std::size_t>(total), std::memory_order_relaxed);
    didMutate = true;
    needRefresh = true;
    AC_LOG_INFO(kModule,
                "Package 0 pruned real family-shared app id(s) (removed_apps=%zu, removed_depots=%zu, "
                "app_vec_total=%u, depot_vec_total=%u).",
                removedApps, removedDepots, total, pkg->depotIdVec.size);
    diag::Record("license_refresh",
                 "family-prune -" + std::to_string(removedApps) + "/d-" + std::to_string(removedDepots));
}

void ApplyHotReloadOnSteamThread(bool& didMutate, bool& needRefresh) {
    auto* pkg = static_cast<PackageInfo*>(g_state.pPackage0.load(std::memory_order_acquire));
    if (!pkg && g_state.pCPackageInfo.load(std::memory_order_acquire) != nullptr) {
        pkg = ac::hooks::AcquirePackage0OnSteamThread();
    }
    if (!pkg || !o_CUtlMemoryGrow) {
        AC_LOG_WARN(kModule, "NotifyLicenseChanged drain: package 0 unavailable; skipping.");
        return;
    }

    const std::vector<AppId> removals = luadata::TakePendingRemovals();
    const std::vector<AppId> additions = luadata::TakePendingAdditions();
    std::vector<AppId> pruneIds = removals;
    const std::vector<AppId> familyRoots = ActiveFamilyRootIds();
    pruneIds.insert(pruneIds.end(), familyRoots.begin(), familyRoots.end());
    if (pruneIds.empty() && additions.empty()) {
        AC_LOG_DEBUG(kModule, "NotifyLicenseChanged drain: no pending changes; skipping.");
        return;
    }

    std::size_t removed = 0;
    RemoveAllFromPackage0(pkg, pkg->appIdVec, pruneIds, removed);
    std::size_t removedDepots = 0;
    RemoveAllFromPackage0(pkg, pkg->depotIdVec, pruneIds, removedDepots);
    if (removedDepots != 0)
        AC_LOG_DEBUG(kModule, "Removed %zu depot/family-root entry(s) from package 0 depotIdVec.", removedDepots);

    const PackageSeedResult result = SeedPackageVectors(pkg, SeedableIds(), "hotreload", true);
    if (!result.apps.complete)
        AC_LOG_WARN(kModule, "Package 0 hot-reload containment incomplete; the next add repairs it.");

    const std::uint32_t total = pkg->appIdVec.size;
    g_state.package0AppCount.store(static_cast<std::size_t>(total), std::memory_order_relaxed);
    didMutate = true;
    needRefresh = true;
    AC_LOG_INFO(kModule, "Package 0 hot-reload applied (+%zu -%zu/d-%zu, total %u).", additions.size(), removed,
                removedDepots, total);
    diag::Record("license_refresh", "+" + std::to_string(additions.size()) + " -" + std::to_string(removed));
}

void RunUpdateCheckOnSteamThread(AppId app) {
    using CheckAppForUpdate_t = bool (*)(void*, AppId);
    void* manager = g_state.overview.clientAppManager.load(std::memory_order_acquire);
    auto check =
        reinterpret_cast<CheckAppForUpdate_t>(g_state.overview.clientCheckAppForUpdate.load(std::memory_order_acquire));
    if (manager != nullptr && check != nullptr) {
        const bool answer = check(manager, app);
        g_state.overview.updateChecks.fetch_add(1, std::memory_order_relaxed);
        AC_LOG_INFO(kModule, "Checked app %u for updates after its pins moved: check answered %d (%s).", app,
                    answer ? 1 : 0, answer ? "up-to-date" : "update-required");
    } else if (check != nullptr) {
        AC_LOG_WARN_ONCE(kModule,
                         "Skipped update check for app %u after Lua reload: clientAppManager not captured yet.", app);
    }
}

// -------------------------------------------------------------------
// Startup package-0 acquisition monitor
//
// Never calls into steamclient directly: while package 0 is unpublished, this
// monitor only enqueues `RequestAcquirePackage0()` once CPackageInfo exists
// and waits for `BMainLoop` to publish it or for the budget to expire.
// -------------------------------------------------------------------
std::atomic<bool> s_retryStarted{false};
std::mutex s_retryMutex;
std::condition_variable s_retryCv;
bool s_retrySignalled = false;

void MarkRetryIdle() { s_retryStarted.store(false, std::memory_order_relaxed); }

void SignalStartupRetry() {
    {
        std::lock_guard<std::mutex> lock(s_retryMutex);
        s_retrySignalled = true;
    }
    s_retryCv.notify_all();
}

void StartupRetryWorker(std::atomic<bool>& stop) {
    const auto deadline =
        std::chrono::steady_clock::now() +
        std::chrono::milliseconds(constants::kPackageRetryMaxAttempts * constants::kPackageRetryIntervalMs);
    int attempts = 0;
    while (std::chrono::steady_clock::now() < deadline) {
        if (stop.load(std::memory_order_relaxed)) {
            MarkRetryIdle();
            return;
        }
        if (g_state.package0Published.load(std::memory_order_acquire) ||
            g_state.licenseRefreshPending.load(std::memory_order_acquire)) {
            if (g_state.package0Published.load(std::memory_order_acquire)) {
                AC_LOG_INFO(kModule, "Package 0 published after %d monitor cycle(s).", attempts + 1);
            } else {
                AC_LOG_INFO(kModule, "Package 0 seeded after %d monitor cycle(s); waiting for CUser capture.",
                            attempts + 1);
            }
            MarkRetryIdle();
            return;
        }

        if (g_state.pCPackageInfo.load(std::memory_order_acquire) != nullptr) {
            RequestAcquirePackage0();
        }
        if (attempts == 0 || attempts % 30 == 0) {
            AC_LOG_INFO(kModule, "Startup monitor #%d: waiting for package 0 on steamclient main thread.",
                        attempts + 1);
        }
        ++attempts;

        const auto wakeAt = (std::min)(deadline, std::chrono::steady_clock::now() +
                                                     std::chrono::milliseconds(constants::kPackageRetryIntervalMs));
        std::unique_lock<std::mutex> lock(s_retryMutex);
        s_retryCv.wait_until(lock, wakeAt, [&] {
            return stop.load(std::memory_order_relaxed) || g_state.package0Published.load(std::memory_order_relaxed) ||
                   g_state.licenseRefreshPending.load(std::memory_order_relaxed) || s_retrySignalled;
        });
        s_retrySignalled = false;
    }
    AC_LOG_WARN(
        kModule,
        "Package 0 was not published after %d monitor cycle(s); on-demand additions remain queued for BMainLoop.",
        attempts);
    MarkRetryIdle();
}

void StartStartupRetry() {
    {
        std::lock_guard<std::mutex> lock(s_retryMutex);
        s_retrySignalled = false;
    }
    bool expected = false;
    if (!s_retryStarted.compare_exchange_strong(expected, true))
        return;
    if (!workers::StartWorker("license_package0_retry", StartupRetryWorker)) {
        s_retryStarted.store(false, std::memory_order_relaxed);
        AC_LOG_ERROR(kModule, "Could not start the package-0 startup monitor worker.");
    }
}

void StopStartupRetry() {
    MarkRetryIdle();
    SignalStartupRetry();
    AC_LOG_DEBUG(kModule, "Startup monitor stop requested; join handled by workers::Shutdown.");
}

struct DrainGuard {
    bool prev;
    DrainGuard() noexcept : prev(t_draining) { t_draining = true; }
    ~DrainGuard() noexcept { t_draining = prev; }
};

} // namespace

SteamLicenseCallScope::SteamLicenseCallScope() noexcept { ++t_steamLicenseDepth; }
SteamLicenseCallScope::~SteamLicenseCallScope() noexcept { --t_steamLicenseDepth; }

void Init(HMODULE steamclient) {
    if (void* a = pattern::ResolveAddress("CUtlMemoryGrow", "steamclient", steamclient)) {
        o_CUtlMemoryGrow = reinterpret_cast<CUtlMemoryGrow_t>(a);
    }
    AC_LOG_INFO(kModule, "Init (grow=%d, refresh_helpers=%d).", o_CUtlMemoryGrow != nullptr,
                ac::hooks::HasLicenseRefreshHelpers() ? 1 : 0);

    // Covers missed LoadPackage(0) / offline launches without calling Steam
    // APIs off-thread; actual acquisition drains on BMainLoop.
    StartStartupRetry();
}

void Shutdown() {
    StopStartupRetry();
    g_state.steamClientThreadId.store(0, std::memory_order_release);
}

void RecordSteamMainThread(const char* source) {
    const std::uint32_t tid = static_cast<std::uint32_t>(GetCurrentThreadId());
    if (tid == 0)
        return;
    std::uint32_t prev = 0;
    if (g_state.steamClientThreadId.compare_exchange_strong(prev, tid, std::memory_order_acq_rel)) {
        AC_LOG_INFO(kModule, "Learned steamclient main thread id=%u (%s).", tid, source ? source : "unknown");
    }
}

void RequestAcquirePackage0() {
    if (g_state.package0Published.load(std::memory_order_acquire) ||
        g_state.licenseRefreshPending.load(std::memory_order_acquire)) {
        return;
    }
    EnqueueWork([](policy::LicenseDispatchQueue& q) { q.RequestAcquirePackage0(); });
}

void RequestSeedPackage0(PackageInfo* package0) {
    if (!package0)
        return;
    g_state.pPackage0.store(package0, std::memory_order_release);
    g_state.package0AppCount.store(static_cast<std::size_t>(package0->appIdVec.size), std::memory_order_relaxed);
    EnqueueWork([](policy::LicenseDispatchQueue& q) { q.RequestAcquirePackage0(); });
    SignalStartupRetry();
}

std::size_t Package0AppCount() { return g_state.package0AppCount.load(std::memory_order_relaxed); }

void RequestAppInPackage0(AppId appId) {
    if (appId == 0)
        return;
    EnqueueWork([appId](policy::LicenseDispatchQueue& q) { q.RequestApp(appId); });
}

void RequestRemoveAppFromPackage0(AppId appId) {
    if (appId == 0)
        return;
    EnqueueWork([appId](policy::LicenseDispatchQueue& q) { q.RequestRemoveApp(appId); });
}

void RequestLicenseRefresh() {
    EnqueueWork([](policy::LicenseDispatchQueue& q) { q.RequestRefresh(); });
}

void RequestUpdateCheck(AppId appId) {
    if (appId == 0)
        return;
    EnqueueWork([appId](policy::LicenseDispatchQueue& q) { q.RequestUpdateCheck(appId); });
}

bool ShouldDeferUiRecheck(std::uint64_t nowMs) {
    std::lock_guard<std::mutex> lock(g_state.licenseQueueMutex);
    return g_state.licenseQueue.ShouldDeferUiRecheck(nowMs, kMaxUiRecheckDeferMs);
}

void OnUserCaptured() {
    if (g_state.licenseRefreshPending.load(std::memory_order_acquire)) {
        AC_LOG_INFO(kModule, "CUser captured with pending package-0 refresh; queuing license refresh.");
        RequestLicenseRefresh();
    }
    SignalStartupRetry();
}

void WakeStartupRetry() {
    if (g_state.pCPackageInfo.load(std::memory_order_acquire) != nullptr &&
        g_state.pPackage0.load(std::memory_order_acquire) == nullptr) {
        RequestAcquirePackage0();
    }
    SignalStartupRetry();
}

void NotifyLicenseChanged() {
    EnqueueWork([](policy::LicenseDispatchQueue& q) { q.RequestHotReload(); });
    AC_LOG_INFO(kModule, "Package 0 hot-reload queued for steamclient main thread.");
}

void TryDrainOnSteamThread() {
    if (!g_state.licenseWorkPending.load(std::memory_order_acquire))
        return;
    if (!policy::CanDrainLicenseWork(IsSteamMainThread(), t_steamLicenseDepth, t_draining))
        return;

    policy::LicenseDispatchBatch batch;
    {
        std::lock_guard<std::mutex> lock(g_state.licenseQueueMutex);
        batch = g_state.licenseQueue.TakeBatch();
        g_state.licenseWorkPending.store(g_state.licenseQueue.HasPendingWork(), std::memory_order_release);
    }
    if (batch.Empty())
        return;

    const DrainGuard drainGuard;

    bool didMutate = false;
    bool publishAttempted = false;
    bool publishSeedChanged = false;
    bool needRefresh = batch.refresh;
    const char* refreshReason = batch.hotReload ? "hotreload" : (batch.acquirePackage0 ? "publish" : "ondemand");

    if (batch.acquirePackage0) {
        auto* pkg = static_cast<PackageInfo*>(g_state.pPackage0.load(std::memory_order_acquire));
        if (!pkg && g_state.pCPackageInfo.load(std::memory_order_acquire) != nullptr) {
            pkg = ac::hooks::AcquirePackage0OnSteamThread();
            if (pkg) {
                g_state.pPackage0.store(pkg, std::memory_order_release);
                g_state.package0AppCount.store(static_cast<std::size_t>(pkg->appIdVec.size), std::memory_order_relaxed);
            }
        }
        if (pkg && !g_state.package0Published.load(std::memory_order_acquire) &&
            !g_state.licenseRefreshPending.load(std::memory_order_acquire)) {
            // Seed every configured id on first acquisition: later on-demand
            // requests already queued before this publish are covered by the same
            // pass.
            const std::vector<AppId> ids = SeedableIds();
            if (!ids.empty()) {
                const PackageSeedResult result = SeedPackageVectors(pkg, ids, "publish", true);
                publishSeedChanged = result.Changed();
                didMutate = didMutate || publishSeedChanged;
            }
            publishAttempted = true;
            if (publishSeedChanged)
                needRefresh = true;
        }
    }

    if (batch.hotReload) {
        ApplyHotReloadOnSteamThread(didMutate, needRefresh);
    }

    if (!publishAttempted) {
        for (const AppId app : batch.apps) {
            if (AddAppToPackage0OnSteamThread(app)) {
                didMutate = true;
                needRefresh = true;
            }
        }
    }

    RemoveAppsFromPackage0OnSteamThread(batch.removeApps, didMutate, needRefresh);

    bool refreshed = true;
    if (needRefresh) {
        refreshed = RefreshLicensesOnSteamThread(refreshReason);
    }

    if (publishAttempted) {
        auto* pkg = static_cast<PackageInfo*>(g_state.pPackage0.load(std::memory_order_acquire));
        const auto step = policy::DecidePublishStep(pkg != nullptr, publishSeedChanged, refreshed);
        g_state.package0Published.store(step == policy::PublishStep::Published, std::memory_order_release);
        g_state.licenseRefreshPending.store(step == policy::PublishStep::RefreshPending, std::memory_order_release);
        if (step == policy::PublishStep::Published) {
            AC_LOG_INFO(kModule, "Package 0 published with every configured .lua id.");
        } else if (step == policy::PublishStep::RefreshPending) {
            AC_LOG_INFO(kModule, "Package 0 seeded with every configured .lua id; awaiting CUser capture to refresh.");
        }
        SignalStartupRetry();
    }

    for (const AppId app : batch.updateCheckApps) {
        RunUpdateCheckOnSteamThread(app);
    }

    AC_LOG_DEBUG(kModule,
                 "Drained license work on steamclient main thread %lu (acquire=%d, hotreload=%d, ondemand=%zu, "
                 "prune=%zu, refresh=%d, update_checks=%zu).",
                 static_cast<unsigned long>(GetCurrentThreadId()), batch.acquirePackage0 ? 1 : 0,
                 batch.hotReload ? 1 : 0, batch.apps.size(), batch.removeApps.size(), needRefresh ? 1 : 0,
                 batch.updateCheckApps.size());

    if (didMutate || needRefresh || publishAttempted || !batch.updateCheckApps.empty())
        status::Write();
}

} // namespace ac::hooks::LicenseManager
