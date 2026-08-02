#include "pch.h"
#include "hooks/license/LicenseManager.h"

#include <atomic>
#include <limits>
#include <mutex>
#include <string>
#include <thread>
#include <unordered_set>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "utils/PatternEngine.h"

namespace ac::hooks::LicenseManager {
    // Forward declarations for public functions defined below, so helpers in
    // the anonymous namespace (e.g. the startup retry thread) can call them.
    void DoStartupInjection();

    namespace {

        constexpr const char* kModule = "License";
        using steam::AppId;
        using steam::PackageInfo;

        // steamclient!CUtlMemory grow helper used to extend package 0's app-id vector.
        using CUtlMemoryGrow_t = void (*)(steam::CUtlVector<AppId>*, int);
        // steamclient!CClientAppManager::ProcessPendingLicenseUpdates(pCUser).
        using ProcessPendingLicenseUpdates_t = bool (*)(void*);
        // steamclient!CClientUser::MarkLicenseAsChanged(pCUser, packageId, reloadAll).
        using MarkLicenseAsChanged_t = std::int64_t(*)(void*, std::uint32_t, bool);

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
                    ++out.presentBefore;
                }
                if (write != read) data[write] = id;
                ++write;
            }
            pkg->appIdVec.size = write;

            out.total = pkg->appIdVec.size;
            std::vector<AppId> missing;
            missing.reserve(expectedIds.size() - out.presentBefore);
            for (AppId id : expectedIds) {
                if (!seen.count(id)) missing.push_back(id);
            }
            out.missingBefore = missing.size();
            out.presentAfter = out.presentBefore;

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
                out.presentAfter += out.appended;
            }

            out.missingAfter = out.expected - out.presentAfter;
            out.total = pkg->appIdVec.size;
            out.complete = out.missingAfter == 0;
            AC_LOG_INFO(kModule,
                "Package0Containment reason=%s expected=%zu present_before=%zu missing_before=%zu duplicates_removed=%zu appended=%zu present_after=%zu missing_after=%zu total=%u complete=%d.",
                reason ? reason : "unknown", out.expected, out.presentBefore,
                out.missingBefore, out.duplicatesRemoved, out.appended,
                out.presentAfter, out.missingAfter, out.total,
                out.complete ? 1 : 0);
            return out;
        }

        bool Ready() {
            return g_state.pCUser.load() && g_state.pCPackageInfo.load() &&
                g_state.pPackage0.load() && o_CUtlMemoryGrow &&
                o_ProcessPendingLicenseUpdates && o_MarkLicenseAsChanged;
        }

        // -------------------------------------------------------------------
        // Package-0 startup retry (A2)
        //
        // DoStartupInjection is normally triggered by LoadPackage (package 0
        // ready) or the first MarkLicenseAsChanged fire (post-login). Both
        // windows can be missed — offline startup, late login, package 0 not
        // usable yet — so a dedicated thread re-attempts the top-up on a
        // throttled cadence until the seed completes or the budget is spent.
        //
        // The thread is module plumbing (ARCHITECTURE.md §allowed-exceptions
        // #2: private lifecycle of a service module), so its control block is
        // module-local, not AetherCoreState. It calls the existing
        // DoStartupInjection() (idempotent, owns s_packageMutationMutex) and
        // observes the shared g_state.package0Seeded atomic.
        // -------------------------------------------------------------------
        std::thread s_retryThread;
        std::atomic<bool> s_retryStop{false};
        std::atomic<bool> s_retryStarted{false};

        void StartupRetryThread() {
            // The budget is time-based, not attempt-based: it must expire even
            // when package 0 never becomes ready (e.g. Steam never loads it),
            // otherwise the thread would spin forever waiting.
            for (int attempts = 0; attempts < constants::kPackageRetryMaxAttempts; ++attempts) {
                if (s_retryStop.load(std::memory_order_relaxed)) return;
                if (g_state.package0Seeded.load(std::memory_order_relaxed)) return;

                // DoStartupInjection only needs package 0 + the grow helper;
                // CUser/license-update resolution is NOT required for the
                // vector top-up itself (the license refresh happens at login
                // via the MarkLicenseAsChanged hook).
                if (g_state.pPackage0.load() && o_CUtlMemoryGrow) {
                    if (attempts == 0 || attempts % 30 == 0) {
                        AC_LOG_INFO(kModule, "Startup retry #%d (package0 ready).", attempts + 1);
                    }
                    DoStartupInjection();
                    if (g_state.package0Seeded.load(std::memory_order_relaxed)) {
                        AC_LOG_INFO(kModule, "Package 0 seeded after %d retry attempt(s).",
                                    attempts + 1);
                        return;
                    }
                } else if (attempts == 0 || attempts % 30 == 0) {
                    AC_LOG_INFO(kModule, "Startup retry #%d: waiting for package 0.", attempts + 1);
                }

                // Sleep the interval in 10 ms steps so shutdown stays snappy.
                constexpr int kTickMs = 10;
                for (int elapsed = 0; elapsed < constants::kPackageRetryIntervalMs &&
                                    !s_retryStop.load(std::memory_order_relaxed);
                     elapsed += kTickMs) {
                    Sleep(kTickMs);
                }
            }
            AC_LOG_WARN(kModule, "Package 0 startup retry gave up after %d attempt(s); "
                                 "a later MarkLicenseAsChanged/login will still top up.",
                        constants::kPackageRetryMaxAttempts);
        }

        void StartStartupRetry() {
            bool expected = false;
            if (!s_retryStarted.compare_exchange_strong(expected, true)) return;
            s_retryStop.store(false, std::memory_order_relaxed);
            s_retryThread = std::thread(StartupRetryThread);
        }

        void StopStartupRetry() {
            s_retryStop.store(true, std::memory_order_relaxed);
            if (s_retryThread.joinable()) s_retryThread.join();
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

        // Cover the missed-window cases: offline startup, late login, package 0
        // not usable at LoadPackage time. No-op after the seed completes.
        StartStartupRetry();
    }

    void Shutdown() {
        StopStartupRetry();
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
            AC_LOG_DEBUG(kModule, "SeedPackage0: no configured depots yet; deferring to startup retry.");
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
        if (!pkg || !o_CUtlMemoryGrow) {
            AC_LOG_DEBUG_ONCE(kModule, "DoStartupInjection skipped: package0=%d grow=%d.",
                              pkg != nullptr, o_CUtlMemoryGrow != nullptr);
            return;
        }

        std::vector<AppId> ids = luadata::AllDepotIds();
        if (ids.empty()) {
            g_state.package0Seeded.store(false);
            AC_LOG_DEBUG_ONCE(kModule, "DoStartupInjection: no configured depots yet.");
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
