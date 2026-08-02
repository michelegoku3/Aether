#include "pch.h"
#include "hooks/steamclient/LicenseHooks.h"

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/HookManager.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "core/SteamTypes.h"
#include "utils/PatternEngine.h"

#include <atomic>
#include <chrono>
#include <cstdint>
#include <mutex>
#include <string>
#include <thread>
#include <unordered_set>

namespace ac::hooks {
    namespace {

        constexpr const char* kModule = "License.Hooks";
        using namespace ac::steam;

        using OptedInMask_t = std::int64_t(*)(void*, std::uint32_t);
        OptedInMask_t o_OptedInMask = nullptr;
        using RequiresLegacyCDKey_t = bool (*)(void*, AppId, std::uint32_t*);
        RequiresLegacyCDKey_t o_RequiresLegacyCDKey = nullptr;
        using IsCloudEnabledForApp_t = bool (*)(void*, AppId);
        IsCloudEnabledForApp_t o_IsCloudEnabledForApp = nullptr;
        using GetRemoteStorageSyncState_t = std::int32_t(*)(void*, AppId);
        GetRemoteStorageSyncState_t o_GetRemoteStorageSyncState = nullptr;
        using CloseAppCloud_t = bool (*)(void*, AppId);
        CloseAppCloud_t o_CloseAppCloud = nullptr;

        // Only keep these, we deliberately NOT hook Evaluate/RunLaunch/RunExit in v3
        // to avoid re-introducing shutdown window. See analysis.

        // -------------------------------------------------------------------
        // Legacy-CD-key suppression summary (aggregated, debounced).
        //
        // RequiresLegacyCDKey fires once per tracked app during login bursts.
        // Logging each app is noise in a single-log setup, so we collect the
        // suppressed ids in a session set and emit ONE line once the burst
        // settles (same debounce pattern as the Ownership unlock summary, and
        // the same shared debounce/tick constants). Re-logs only when the set
        // of unique ids grows, so idle sessions stay silent.
        // -------------------------------------------------------------------
        std::mutex s_cdKeyMutex;
        std::unordered_set<AppId> s_cdKeySuppressed;
        std::size_t s_cdKeyLastLoggedSize = 0;
        std::thread s_cdKeyThread;
        std::atomic<bool> s_cdKeyStop{false};
        std::atomic<bool> s_cdKeyStarted{false};
        std::atomic<std::int64_t> s_cdKeyDeadline{0};

        std::int64_t CdKeySteadyNowMs() {
            return std::chrono::duration_cast<std::chrono::milliseconds>(
                std::chrono::steady_clock::now().time_since_epoch()).count();
        }

        void LogCdKeySummary() {
            std::size_t unique = 0;
            {
                std::lock_guard<std::mutex> lock(s_cdKeyMutex);
                unique = s_cdKeySuppressed.size();
                if (unique == s_cdKeyLastLoggedSize) return;
                s_cdKeyLastLoggedSize = unique;
            }
            AC_LOG_INFO(kModule, "RequiresLegacyCDKey suppressed for %zu unique app(s).",
                        unique);
        }

        void CdKeySummaryThread() {
            for (;;) {
                if (s_cdKeyStop.load(std::memory_order_relaxed)) return;
                const std::int64_t deadline = s_cdKeyDeadline.load(std::memory_order_relaxed);
                if (deadline != 0 && CdKeySteadyNowMs() >= deadline) {
                    s_cdKeyDeadline.store(0, std::memory_order_relaxed);
                    LogCdKeySummary();
                }
                Sleep(constants::kUnlockSummaryTickMs);
            }
        }

        void ArmCdKeySummary() {
            s_cdKeyDeadline.store(CdKeySteadyNowMs() + constants::kUnlockSummaryDebounceMs,
                                  std::memory_order_relaxed);
            bool expected = false;
            if (s_cdKeyStarted.compare_exchange_strong(expected, true)) {
                s_cdKeyStop.store(false, std::memory_order_relaxed);
                s_cdKeyThread = std::thread(CdKeySummaryThread);
            }
        }

        void StopCdKeySummary() {
            s_cdKeyStop.store(true, std::memory_order_relaxed);
            if (s_cdKeyThread.joinable()) s_cdKeyThread.join();
        }

        void RecordCdKeySuppressed(AppId app) {
            {
                std::lock_guard<std::mutex> lock(s_cdKeyMutex);
                s_cdKeySuppressed.insert(app);
            }
            ArmCdKeySummary();
        }

        // Cloud-gate log dedup + counters live in g_state.cloudGate
        // (centralized state; see AetherCoreState.h).

        enum class ERemoteSyncState : int32_t {
            Disabled = 0,
            Unknown = 1,
            Synchronized = 2,
        };

        struct CloudPolicy {
            bool tracked = false, managed = false, owned = false, familyShared = false, block = false;
            const char* cls = "untracked";
        };
        const char* OwnershipClass(bool t, bool m, bool o, bool f) {
            if (f) return "family-shared";
            if (o) return "steam-provided";
            if (m) return "managed-unowned";
            if (t) return "lua-tracked";
            return "untracked";
        }
        CloudPolicy GetPolicy(AppId id) {
            CloudPolicy p;
            p.managed = luadata::HasDepot(id);
            p.tracked = luadata::IsConfigured(id);
            p.owned = luadata::IsOwned(id);
            p.familyShared = luadata::IsFamilyShared(id);
            p.block = p.managed && !p.owned && !p.familyShared;
            p.cls = OwnershipClass(p.tracked, p.managed, p.owned, p.familyShared);
            return p;
        }
        uint64_t LogKey(AppId id, const char* stage) {
            uint64_t h = (uint64_t)id << 32;
            if (!stage) return h;
            while (*stage) h = h * 131 + (unsigned char)*stage++;
            return h;
        }
        bool ShouldBlock(AppId id, const char* stage) {
            auto pol = GetPolicy(id);
            if (!pol.block) {
                if (pol.tracked || pol.managed || pol.owned || pol.familyShared) {
                    const char* s = stage ? stage : "unknown";
                    AC_LOG_DEBUG(kModule, "CloudGate %s appid=%u class=%s passthrough", s, id, pol.cls);
                }
                return false;
            }
            std::lock_guard<std::mutex> lk(g_state.cloudGate.logMutex);
            if (g_state.cloudGate.syncBlockedLogged.insert(LogKey(id, stage)).second) {
                const char* s = stage ? stage : "unknown";
                AC_LOG_INFO(kModule, "CloudGate %s appid=%u class=%s BLOCKED", s, id, pol.cls);
                diag::Record("cloud_gate_blocked", std::string(s) + " appid=" + std::to_string(id) + " class=" + pol.cls);
            }
            return true;
        }

        // Address data is owned by PatternEngine. It merges the per-build TOML
        // with the central hardcoded fallback table, so this feature module does
        // not contain a second scanner or a second source of truth.
        template<typename Fn>
        bool HookPattern(const std::string& name, const std::string& modName, HMODULE mod,
                         Fn& orig, Fn detour) {
            void* target = pattern::ResolveAddress(name, modName, mod);
            if (!target) {
                g_state.hookManager.RecordMissed(name);
                return false;
            }
            g_state.hookManager.RegisterHook(name, target,
                                              reinterpret_cast<void**>(&orig),
                                              reinterpret_cast<void*>(detour));
            AC_LOG_DEBUG(kModule, "Hook queued %s @ %p", name.c_str(), target);
            return true;
        }

        // --- hooks ---
        std::int64_t h_OptedInMask(void* self, std::uint32_t appId) {
            AppId real = g_state.onlineFixRealAppId.load();
            if (appId == constants::kSpacewarAppId && real != 0) return o_OptedInMask(self, real);
            return o_OptedInMask(self, appId);
        }
        bool h_RequiresLegacyCDKey(void* self, AppId appId, std::uint32_t* out) {
            if (luadata::HasDepot(appId)) {
                if (out) *out = 0;
                RecordCdKeySuppressed(appId);
                return false;
            }
            return o_RequiresLegacyCDKey(self, appId, out);
        }
        bool h_IsCloudEnabledForApp(void* rs, AppId appId) {
            auto pol = GetPolicy(appId);
            if (pol.tracked && pol.familyShared) {
                std::lock_guard<std::mutex> lk(g_state.cloudGate.logMutex);
                if (g_state.cloudGate.familyLogged.insert(appId).second) {
                    AC_LOG_INFO(kModule, "IsCloud appid=%u family-shared ALLOW", appId);
                    diag::Record("cloud_family_allow", "appid=" + std::to_string(appId));
                }
                return true;
            }
            if (!pol.tracked || pol.owned) {
                if (!o_IsCloudEnabledForApp) return true;
                return o_IsCloudEnabledForApp(rs, appId);
            }
            if (pol.managed) {
                std::lock_guard<std::mutex> lk(g_state.cloudGate.logMutex);
                if (g_state.cloudGate.blockedLogged.insert(appId).second) {
                    ++g_state.cloudGate.totalBlocks;
                    AC_LOG_INFO(kModule, "IsCloud appid=%u BLOCKED managed-unowned total=%llu", appId, (unsigned long long)g_state.cloudGate.totalBlocks);
                    diag::Record("cloud_blocked", "appid=" + std::to_string(appId));
                }
                return false;
            }
            if (!o_IsCloudEnabledForApp) return true;
            return o_IsCloudEnabledForApp(rs, appId);
        }
        std::int32_t h_GetRemoteStorageSyncState(void* rs, AppId appId) {
            if (ShouldBlock(appId, "state")) {
                // Try both Disabled and Synchronized – Disabled is semantically correct, 
                // but some Steam builds treat Disabled as "needs check" and show window.
                // We try Synchronized=2 which definitively says "up to date, no wait"
                auto res = (int32_t)ERemoteSyncState::Synchronized;
                // If you still see window, try Disabled=0
                // auto res=(int32_t)ERemoteSyncState::Disabled;
                AC_LOG_INFO(kModule, "GetSyncState appid=%u -> %d Synchronized BLOCKED", appId, res);
                diag::Record("cloud_state_blocked", "appid=" + std::to_string(appId) + " result=" + std::to_string(res));
                return res;
            }
            if (!o_GetRemoteStorageSyncState) return (int32_t)ERemoteSyncState::Unknown;
            return o_GetRemoteStorageSyncState(rs, appId);
        }
        bool h_CloseAppCloud(void* rs, AppId appId) {
            if (ShouldBlock(appId, "close")) {
                AC_LOG_INFO(kModule, "CloseAppCloud appid=%u -> true immediate (fixes shutdown hang)", appId);
                diag::Record("cloud_close_blocked", "appid=" + std::to_string(appId));
                return true;
            }
            if (!o_CloseAppCloud) return true;
            return o_CloseAppCloud(rs, appId);
        }

    } // anon
    void RegisterLicenseHooks(HMODULE diversion) {
        if (!diversion) { AC_LOG_ERROR(kModule, "diversion null"); return; }
        AC_LOG_INFO(kModule, "Registering FIXED v3 - minimal gates to avoid shutdown window");

        // PatternEngine merges the build TOML with PatternFallbacks.h. TOML
        // entries take precedence; missing entries use the central fallback
        // table and are reported in status/log diagnostics.
        HookPattern("OptedInMask", "steamclient", diversion, o_OptedInMask, h_OptedInMask);
        HookPattern("RequiresLegacyCDKey", "steamclient", diversion, o_RequiresLegacyCDKey, h_RequiresLegacyCDKey);
        HookPattern("IsCloudEnabledForApp", "steamclient", diversion, o_IsCloudEnabledForApp, h_IsCloudEnabledForApp);
        HookPattern("GetRemoteStorageSyncState", "steamclient", diversion, o_GetRemoteStorageSyncState, h_GetRemoteStorageSyncState);
        HookPattern("CloseAppCloud", "steamclient", diversion, o_CloseAppCloud, h_CloseAppCloud);

        // Final per-hook status is reported by HookManager::InstallAll() and
        // StatusWriter reads g_state.hookManager directly — no local summary
        // here, mirroring the other Register*Hooks modules.
    }

    void ShutdownLicenseHooks() {
        StopCdKeySummary();
    }
} // namespace ac::hooks
