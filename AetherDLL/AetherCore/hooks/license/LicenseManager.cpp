#include "pch.h"
#include "hooks/license/LicenseManager.h"

#include <limits>
#include <mutex>
#include <string>
#include <unordered_set>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
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
    std::size_t present = 0;
    std::size_t duplicatesRemoved = 0;
    std::size_t missing = 0;
    std::size_t appended = 0;
    std::uint32_t total = 0;
    bool complete = false;
};

bool RemoveAllFromPackage0(PackageInfo* pkg, const std::vector<AppId>& ids,
                           std::size_t& removed) {
    if (!pkg || !pkg->appIdVec.mem.memory || ids.empty()) return true;
    std::unordered_set<AppId> removeSet;
    removeSet.reserve(ids.size());
    for (AppId id : ids) {
        if (id != 0) removeSet.insert(id);
    }

    AppId* data = pkg->appIdVec.mem.memory;
    std::uint32_t write = 0;
    for (std::uint32_t read = 0; read < pkg->appIdVec.size; ++read) {
        if (removeSet.count(data[read])) {
            ++removed;
            continue;
        }
        if (write != read) data[write] = data[read];
        ++write;
    }
    pkg->appIdVec.size = write;
    return true;
}

// Ensures each currently managed AppID occurs exactly once in package 0.
// Caller holds s_packageMutationMutex. Unmanaged Steam entries are preserved.
PackageContainmentResult EnsurePackageContains(PackageInfo* pkg,
                                               const std::vector<AppId>& ids,
                                               const char* reason) {
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
        if (id != 0 && expected.insert(id).second) expectedIds.push_back(id);
    }
    out.expected = expectedIds.size();

    if (pkg->appIdVec.size > 0 && !pkg->appIdVec.mem.memory) {
        AC_LOG_ERROR(kModule, "Package0Containment reason=%s has null vector memory.",
                     reason ? reason : "unknown");
        return out;
    }
    if (pkg->appIdVec.size > pkg->appIdVec.mem.allocationCount) {
        AC_LOG_ERROR(kModule, "Package0Containment reason=%s size=%u exceeds capacity=%u.",
                     reason ? reason : "unknown", pkg->appIdVec.size,
                     pkg->appIdVec.mem.allocationCount);
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
            ++out.present;
        }
        if (write != read) data[write] = id;
        ++write;
    }
    pkg->appIdVec.size = write;

    out.total = pkg->appIdVec.size;
    std::vector<AppId> missing;
    missing.reserve(expectedIds.size() - out.present);
    for (AppId id : expectedIds) {
        if (!seen.count(id)) missing.push_back(id);
    }
    out.missing = missing.size();

    if (!missing.empty()) {
        if (missing.size() > static_cast<std::size_t>(std::numeric_limits<int>::max())) {
            AC_LOG_ERROR(kModule, "Package0Containment reason=%s missing list too large.",
                         reason ? reason : "unknown");
            return out;
        }
        const std::uint32_t oldSize = pkg->appIdVec.size;
        const std::size_t required = static_cast<std::size_t>(oldSize) + missing.size();
        if (required > std::numeric_limits<std::uint32_t>::max()) {
            AC_LOG_ERROR(kModule, "Package0Containment reason=%s size overflow.",
                         reason ? reason : "unknown");
            return out;
        }
        o_CUtlMemoryGrow(&pkg->appIdVec, static_cast<int>(missing.size()));
        if (!pkg->appIdVec.mem.memory ||
            pkg->appIdVec.mem.allocationCount < required) {
            AC_LOG_ERROR(kModule, "CUtlMemoryGrow failed for package 0 (reason=%s).",
                         reason ? reason : "unknown");
            return out;
        }
        for (std::size_t i = 0; i < missing.size(); ++i) {
            pkg->appIdVec.mem.memory[oldSize + i] = missing[i];
        }
        pkg->appIdVec.size = static_cast<std::uint32_t>(required);
        out.appended = missing.size();
        out.missing = 0;
    }

    out.total = pkg->appIdVec.size;
    out.complete = out.missing == 0;
    AC_LOG_INFO(kModule,
                "Package0Containment reason=%s expected=%zu present=%zu duplicates_removed=%zu missing=%zu appended=%zu total=%u complete=%d.",
                reason ? reason : "unknown", out.expected, out.present,
                out.duplicatesRemoved, out.missing, out.appended, out.total,
                out.complete ? 1 : 0);
    return out;
}

bool Ready() {
    return g_state.pCUser.load() && g_state.pCPackageInfo.load() &&
           g_state.pPackage0.load() && o_CUtlMemoryGrow &&
           o_ProcessPendingLicenseUpdates && o_MarkLicenseAsChanged;
}

}  // namespace

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
}

void SeedPackage0(PackageInfo* package0) {
    std::lock_guard<std::mutex> lock(s_packageMutationMutex);
    // Single store point: SeedPackage0 owns the pPackage0 publication so the
    // pointer and the seeding stay atomic w.r.t. other mutators.
    g_state.pPackage0.store(package0);

    std::vector<AppId> ids = luadata::AllDepotIds();
    if (ids.empty()) {
        // Lua parsing may not have run yet; DoStartupInjection will catch up.
        g_state.package0Seeded.store(false);
        return;
    }
    const PackageContainmentResult result = EnsurePackageContains(package0, ids, "loadpackage");
    g_state.package0Seeded.store(result.complete);
    if (!result.complete) {
        AC_LOG_WARN(kModule, "Package 0 seed incomplete; startup retry remains enabled.");
    }
}

void DoStartupInjection() {
    std::lock_guard<std::mutex> lock(s_packageMutationMutex);
    // Reads the package-0 pointer from g_state, so the deferred path works too:
    // when LoadPackage saw package 0 with status != 0 the hook stored the
    // pointer without seeding, and this top-up can now really run.
    auto* pkg = static_cast<PackageInfo*>(g_state.pPackage0.load());
    if (!pkg || !o_CUtlMemoryGrow) return;

    std::vector<AppId> ids = luadata::AllDepotIds();
    if (ids.empty()) {
        g_state.package0Seeded.store(false);
        return;
    }
    const PackageContainmentResult result = EnsurePackageContains(pkg, ids, "startup");
    g_state.package0Seeded.store(result.complete);
    if (!result.complete) {
        AC_LOG_WARN(kModule, "Package 0 startup containment incomplete; another retry may repair it.");
    }
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
        if (removals.empty() && additions.empty()) return;

        auto* pkg = static_cast<PackageInfo*>(g_state.pPackage0.load());
        if (!pkg) {
            AC_LOG_WARN(kModule, "NotifyLicenseChanged: package 0 unavailable; skipping.");
            return;
        }
        RemoveAllFromPackage0(pkg, removals, removed);
        // Reconcile against the complete current Lua set, not only additions.
        // This repairs duplicates left by an older build and makes reloads
        // idempotent even when filesystem events are repeated.
        const PackageContainmentResult result =
            EnsurePackageContains(pkg, luadata::AllDepotIds(), "hotreload");
        g_state.package0Seeded.store(result.complete);

        total = pkg->appIdVec.size;
        cUser = g_state.pCUser.load();
        markLicenseAsChanged = o_MarkLicenseAsChanged;
        processPendingLicenseUpdates = o_ProcessPendingLicenseUpdates;
    }

    // Ask Steam to re-evaluate package state outside s_packageMutationMutex. These calls can
    // synchronously trigger callbacks/hooks; keeping our mutation lock held here
    // would make re-entrancy and deadlocks much more likely.
    markLicenseAsChanged(cUser, 0, true);
    processPendingLicenseUpdates(cUser);

    AC_LOG_INFO(kModule, "License refresh: +%zu -%zu (total %u).", additions.size(), removed, total);
    diag::Record("license_refresh", "+" + std::to_string(additions.size()) +
                                      " -" + std::to_string(removed));
}

}  // namespace ac::hooks::LicenseManager
