#include "pch.h"
#include "hooks/license/LicenseManager.h"

#include <mutex>
#include <string>
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

// Appends ids to package 0's vector. Caller holds s_packageMutationMutex and has verified the
// grow helper is available. After the grow call we verify the capacity increased
// enough — if the allocation silently failed, proceeding would write past the
// buffer and corrupt memory.
void AppendToPackage0(PackageInfo* pkg, const std::vector<AppId>& ids) {
    if (ids.empty()) return;
    const std::uint32_t oldSize = pkg->appIdVec.size;
    o_CUtlMemoryGrow(&pkg->appIdVec, static_cast<int>(ids.size()));
    // Verify the grow actually expanded capacity enough; if not, bail to
    // prevent out-of-bounds writes.
    if (pkg->appIdVec.mem.allocationCount < oldSize + ids.size()) {
        AC_LOG_ERROR(kModule, "CUtlMemoryGrow failed to expand package 0; injection skipped.");
        return;
    }
    for (std::size_t i = 0; i < ids.size(); ++i) {
        pkg->appIdVec.mem.memory[oldSize + i] = ids[i];
    }
    pkg->appIdVec.size = oldSize + static_cast<std::uint32_t>(ids.size());
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

    // Inject once. Double injection would inflate ExistInPackageNums and make
    // CheckAppOwnership treat fake-owned apps as genuinely owned.
    if (g_state.package0Seeded.load()) return;
    if (!o_CUtlMemoryGrow) {
        AC_LOG_WARN(kModule, "Cannot seed package 0: CUtlMemoryGrow unresolved.");
        return;
    }
    std::vector<AppId> ids = luadata::AllDepotIds();
    if (ids.empty()) {
        // Lua parsing may not have run yet; DoStartupInjection will catch up.
        return;
    }
    AppendToPackage0(package0, ids);
    g_state.package0Seeded.store(true);
    AC_LOG_INFO(kModule, "Seeded package 0 with %zu app id(s) (total %u).", ids.size(),
                package0->appIdVec.size);
}

void DoStartupInjection() {
    std::lock_guard<std::mutex> lock(s_packageMutationMutex);
    // Reads the package-0 pointer from g_state, so the deferred path works too:
    // when LoadPackage saw package 0 with status != 0 the hook stored the
    // pointer without seeding, and this top-up can now really run.
    auto* pkg = static_cast<PackageInfo*>(g_state.pPackage0.load());
    if (g_state.package0Seeded.load() || !pkg || !o_CUtlMemoryGrow) return;

    std::vector<AppId> ids = luadata::AllDepotIds();
    if (ids.empty()) return;
    AppendToPackage0(pkg, ids);
    g_state.package0Seeded.store(true);
    AC_LOG_INFO(kModule, "Startup injection added %zu app id(s) (total %u).", ids.size(),
                pkg->appIdVec.size);
}

void NotifyLicenseChanged() {
    std::vector<AppId> removals;
    std::vector<AppId> additions;
    std::uint32_t removed = 0;
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
        for (AppId id : removals) {
            if (pkg->appIdVec.FindAndFastRemove(id)) ++removed;
        }
        AppendToPackage0(pkg, additions);

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

    AC_LOG_INFO(kModule, "License refresh: +%zu -%u (total %u).", additions.size(), removed, total);
    diag::Record("license_refresh", "+" + std::to_string(additions.size()) +
                                      " -" + std::to_string(removed));
}

}  // namespace ac::hooks::LicenseManager
