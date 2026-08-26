#include "pch.h"
#include "hooks/steamclient/OwnershipHooks.h"

#include <atomic>
#include <chrono>
#include <limits>
#include <mutex>
#include <sstream>
#include <string>
#include <thread>
#include <unordered_set>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/HookManager.h"
#include "hooks/license/LicenseManager.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "core/SteamTypes.h"
#include "utils/SmartIdLog.h"

namespace ac::hooks {
    namespace {

        constexpr const char* kModule = "Ownership";
        using namespace ac::steam;

        // ---- Hook signatures -------------------------------------------------------
        using CheckAppOwnership_t = bool (*)(void*, AppId, AppOwnership*);
        using MarkLicenseAsChanged_t = std::int64_t(*)(void*, std::uint32_t, bool);
        using GetPackageInfo_t = void* (*)(void*, std::uint32_t, std::int64_t);
        using GetSubscribedApps_t = std::uint32_t(*)(void*, AppId*, std::uint32_t, std::uint8_t);
        using SendCallbackToPipe_t = bool (*)(void*, HSteamPipe, HSteamUser, int, void*, int);
        using LoadPackage_t = bool (*)(PackageInfo*, std::uint8_t*, std::int32_t, void*);

        CheckAppOwnership_t o_CheckAppOwnership = nullptr;
        MarkLicenseAsChanged_t o_MarkLicenseAsChanged = nullptr;
        GetPackageInfo_t o_GetPackageInfo = nullptr;
        GetSubscribedApps_t o_GetSubscribedApps = nullptr;
        SendCallbackToPipe_t o_SendCallbackToPipe = nullptr;
        LoadPackage_t o_LoadPackage = nullptr;

        // Smart debounced once-per-app ownership outcome logs (legit/family are rare;
        // the "unlocked" outcome is aggregated per-file instead of per-appid).
        logutil::SmartIdLog s_logLegit(kModule, "Legit-owned AppIds");
        logutil::SmartIdLog s_logFamily(kModule, "Family-shared AppIds");

        // ---------------------------------------------------------------------------
        // AetherOnline achievement callback rewrite (single source of truth).
        //
        // Steam delivers user-stats callbacks (UserStatsReceived 1101, UserStatsStored
        // 1102, UserAchievementStored 1103, UserAchievementIconFetched 1109) with the
        // REAL app id in m_nGameID when the stats store was processed for the real
        // game (see the wire-layer 480→real rewrites and the IClientUserStats
        // stats-scope). Under an AetherOnline session the game process is masked as
        // Spacewar/480 and its Steamworks handlers expect m_nGameID == 480, so the
        // payload must be rewritten to 480 before dispatch or the game ignores the
        // callback.
        //
        // Returns true when the payload was rewritten; leaves it untouched when no
        // session is active, the callback is not achievement-related, or the payload
        // does not carry the real app id.
        // ---------------------------------------------------------------------------
        bool RewriteAetherOnlineCallbackGameId(int cb, void* data, int size) {
            const AppId realApp = g_state.aetherOnlineRealAppId.load(std::memory_order_acquire);
            if (realApp == 0 || realApp == constants::kSpacewarAppId) return false;
            if (!constants::achievement_cb::IsAchievementCallback(cb)) return false;
            if (data == nullptr || size < static_cast<int>(sizeof(std::uint64_t))) return false;

            auto* pGameId = static_cast<std::uint64_t*>(data);
            const AppId currentApp = static_cast<AppId>(*pGameId & constants::kGameIdAppIdMask);
            if (currentApp != realApp) return false;

            *pGameId = (*pGameId & ~static_cast<std::uint64_t>(constants::kGameIdAppIdMask))
                | static_cast<std::uint64_t>(constants::kSpacewarAppId);
            AC_LOG_DEBUG_ONCE(kModule, "AetherOnline: callback cb=%d m_nGameID %u -> %u.",
                cb, realApp, constants::kSpacewarAppId);
            return true;
        }

        // ---------------------------------------------------------------------------
        // AetherOnline callback delivery policy — whether an achievement callback needs a
        // second copy with m_nGameID rewritten to the Spacewar/480 mask for the game's
        // own Steamworks handlers.
        //
        // UserAchievementStored (1103) is the exception: it is the event that drives
        // the Steam overlay unlock toast, which is keyed to the REAL app id. Delivering
        // a second (480) copy would make the overlay show the same toast twice — the
        // duplicate notification. The game's achievement state is updated through the
        // stats store, not via 1103, so the game needs no 480 copy of it.
        // ---------------------------------------------------------------------------
        bool NeedsRewrittenGameCopy(int cb) {
            return cb != constants::achievement_cb::kUserAchievementStored;
        }

        // ---------------------------------------------------------------------------
        // Unlock summary (replaces the old per-appid "Unlocked AppIds" spam).
        //
        // Every spoofed ownership outcome is recorded into a session set. A debounce
        // thread then emits ONE line per .lua file once the burst settles:
        //   INFO  "Unlocked all AppID for 1144200.lua."          (all expected ok)
        //   WARN  "Not unlocked for 1144200.lua: 1144201, ..."   (single message, ids)
        //
        // Expected ids per file = the ids the file contributes that still need spoofing
        // (luadata::HasDepot — configured, not genuinely owned, not family-shared).
        // The debounce re-arms on every unlock, so late checks (game launch, hot-reload)
        // produce a fresh summary instead of being missed. The thread only reads
        // LuaData (shared locks) and writes logs — it never touches Steam internals.
        // ---------------------------------------------------------------------------
        std::mutex s_unlockMutex;
        std::unordered_set<AppId> s_unlockedAppIds;
        std::thread s_summaryThread;
        std::atomic<bool> s_summaryStop{ false };
        std::atomic<bool> s_summaryStarted{ false };
        std::atomic<std::int64_t> s_summaryDeadline{ 0 };
        // Last emitted outcome fingerprint: "per-file missing list" as a stable string.
        // The summary is re-logged only when this fingerprint changes (or on the very
        // first emission), so repeated bursts (login, hot-reload, launch) stay quiet.
        std::string s_lastSummaryFingerprint;
        bool s_hasEmitted = false;

        std::int64_t SteadyNowMs() {
            return std::chrono::duration_cast<std::chrono::milliseconds>(
                std::chrono::steady_clock::now().time_since_epoch()).count();
        }

        std::string FileBaseName(const std::string& path) {
            const std::size_t slash = path.find_last_of("\\/");
            return slash == std::string::npos ? path : path.substr(slash + 1);
        }

        void LogUnlockedSummary() {
            std::unordered_set<AppId> unlocked;
            {
                std::lock_guard<std::mutex> lock(s_unlockMutex);
                unlocked = s_unlockedAppIds;
            }

            const auto byFile = luadata::ConfiguredIdsByFile();
            std::size_t expectedTotal = 0;
            std::size_t unlockedTotal = 0;
            std::size_t files = 0;
            // Stable fingerprint of the outcome; identical outcomes are logged once.
            std::ostringstream fingerprint;

            for (const auto& [path, ids] : byFile) {
                std::vector<AppId> expected;
                expected.reserve(ids.size());
                for (AppId id : ids) {
                    // Only ids that still require spoofing; owned/family-shared ids are
                    // legitimately covered by Steam and must not be reported as missing.
                    if (luadata::HasDepot(id)) expected.push_back(id);
                }
                if (expected.empty()) continue;
                ++files;
                expectedTotal += expected.size();

                std::vector<AppId> missing;
                for (AppId id : expected) {
                    if (unlocked.count(id)) {
                        ++unlockedTotal;
                    }
                    else {
                        missing.push_back(id);
                    }
                }

                const std::string base = FileBaseName(path);
                fingerprint << base << ':';
                if (missing.empty()) {
                    fingerprint << "OK;";
                }
                else {
                    // Full missing-id list in the fingerprint so a change in which ids
                    // are missing (even with the same count) re-logs the summary.
                    for (std::size_t i = 0; i < missing.size(); ++i) {
                        if (i) fingerprint << ',';
                        fingerprint << missing[i];
                    }
                    fingerprint << ';';
                }
            }

            // Emit only when the outcome changed since the last emission. This stops
            // the endless re-logging while Steam re-checks ownership on every burst.
            {
                std::lock_guard<std::mutex> lock(s_unlockMutex);
                const std::string fp = fingerprint.str();
                if (s_hasEmitted && fp == s_lastSummaryFingerprint) {
                    return;
                }
                s_lastSummaryFingerprint = fp;
                s_hasEmitted = true;
            }

            // Fingerprint changed (or first time): emit the per-file lines + totals.
            for (const auto& [path, ids] : byFile) {
                std::vector<AppId> missing;
                for (AppId id : ids) {
                    if (luadata::HasDepot(id) && !unlocked.count(id)) missing.push_back(id);
                }
                if (missing.empty()) {
                    AC_LOG_INFO(kModule, "Unlocked all AppID for %s.", FileBaseName(path).c_str());
                }
                else {
                    std::ostringstream list;
                    for (std::size_t i = 0; i < missing.size(); ++i) {
                        if (i) list << ", ";
                        list << missing[i];
                    }
                    AC_LOG_WARN(kModule, "Not unlocked for %s: %s.", FileBaseName(path).c_str(),
                        list.str().c_str());
                }
            }

            AC_LOG_DEBUG(kModule, "Unlock summary: %zu/%zu appids across %zu file(s).",
                unlockedTotal, expectedTotal, files);
        }

        void UnlockSummaryThread() {
            for (;;) {
                if (s_summaryStop.load(std::memory_order_relaxed)) return;
                const std::int64_t deadline = s_summaryDeadline.load(std::memory_order_relaxed);
                if (deadline != 0 && SteadyNowMs() >= deadline) {
                    s_summaryDeadline.store(0, std::memory_order_relaxed);
                    LogUnlockedSummary();
                }
                Sleep(constants::kUnlockSummaryTickMs);
            }
        }

        void ArmUnlockSummary() {
            s_summaryDeadline.store(SteadyNowMs() + constants::kUnlockSummaryDebounceMs,
                std::memory_order_relaxed);
            bool expected = false;
            if (s_summaryStarted.compare_exchange_strong(expected, true)) {
                s_summaryStop.store(false, std::memory_order_relaxed);
                s_summaryThread = std::thread(UnlockSummaryThread);
            }
        }

        void StopUnlockSummary() {
            s_summaryStop.store(true, std::memory_order_relaxed);
            if (s_summaryThread.joinable()) s_summaryThread.join();
        }

        void RecordUnlocked(AppId app) {
            // Session set only: per-appid lines are pure noise in a single-log setup.
            // The per-file summary (LogUnlockedSummary) is the only output.
            {
                std::lock_guard<std::mutex> lock(s_unlockMutex);
                s_unlockedAppIds.insert(app);
            }
            ArmUnlockSummary();
        }

        enum class OwnershipOutcome {
            LegitOwned,
            FamilyShared,
            Unlocked,
        };

        void RecordOwnershipOutcome(AppId app, OwnershipOutcome outcome) {
            switch (outcome) {
            case OwnershipOutcome::LegitOwned:
                s_logLegit.Record(app);
                break;
            case OwnershipOutcome::FamilyShared:
                s_logFamily.Record(app);
                break;
            case OwnershipOutcome::Unlocked:
                RecordUnlocked(app);
                break;
            }
        }

        // ---- Hook bodies -----------------------------------------------------------

        bool h_CheckAppOwnership(void* self, AppId app, AppOwnership* out) {
            bool result = o_CheckAppOwnership(self, app, out);
            if (!out || !luadata::HasDepot(app)) return result;

            const auto originalReleaseState = out->releaseState;
            const auto originalExistInPackageNums = out->existInPackageNums;
            const bool originalBorrowed = out->borrowed;
            const bool originalFamilyShared = out->familyShared;
            const bool released = originalReleaseState == AppReleaseState::Released;
            const bool steamProvided = result && originalExistInPackageNums > 1 && released;
            const bool familyShared = steamProvided && (originalBorrowed || originalFamilyShared);

            // Match LumaCore's conservative classification: a Lua app is considered
            // Steam-provided only when Steam reports a released app present in more than
            // one package. This avoids the over-broad "if (result)" path that can make
            // every injected Lua app disappear, and it also avoids treating Steam common
            // redistributables/package-tool entries as user-owned games just because
            // they have a non-zero package id.
            if (steamProvided) {
                if (familyShared) {
                    luadata::MarkFamilyShared(app);
                    RecordOwnershipOutcome(app, OwnershipOutcome::FamilyShared);
                }
                else {
                    luadata::MarkOwned(app);
                    RecordOwnershipOutcome(app, OwnershipOutcome::LegitOwned);
                }
                AC_LOG_DEBUG_ONCE(kModule,
                    "CheckAppOwnership: %u steam-provided class=%s result=%d exist=%u release=%u borrowed=%d familyShared=%d.",
                    app, familyShared ? "family-shared" : "owned", result ? 1 : 0,
                    originalExistInPackageNums, static_cast<unsigned>(originalReleaseState),
                    originalBorrowed ? 1 : 0, originalFamilyShared ? 1 : 0);
                return result;
            }

            // Otherwise spoof ownership via our synthetic package 0.
            out->packageId = 0;
            out->releaseState = AppReleaseState::Released;
            out->freeLicense = false;
            out->ownsLicense = true;
            RecordOwnershipOutcome(app, OwnershipOutcome::Unlocked);
            return true;
        }

        std::int64_t h_MarkLicenseAsChanged(void* self, std::uint32_t packageId, bool reloadAll) {
            // Atomic store: the captured pointer is read from other threads (IPC hooks).
            // LicenseManager reads it straight from g_state (single source of truth).
            void* prev = nullptr;
            if (g_state.pCUser.compare_exchange_strong(prev, self)) {
                AC_LOG_INFO(kModule, "Captured CUser pointer 0x%p.", self);
                // First fire is post-login: top up package 0 with anything parsed after
                // LoadPackage ran.
                LicenseManager::DoStartupInjection();
            }
            return o_MarkLicenseAsChanged(self, packageId, reloadAll);
        }

        void* h_GetPackageInfo(void* self, std::uint32_t packageId, std::int64_t p3) {
            // Atomic compare-and-swap: capture the pointer exactly once.
            void* prev = nullptr;
            if (g_state.pCPackageInfo.compare_exchange_strong(prev, self)) {
                AC_LOG_INFO(kModule, "Captured CPackageInfo pointer 0x%p.", self);
            }
            return o_GetPackageInfo(self, packageId, p3);
        }


        std::uint32_t h_GetSubscribedApps(void* self, AppId* appList, std::uint32_t size,
            std::uint8_t unknownFlag) {
            std::uint32_t count = o_GetSubscribedApps(self, appList, size, unknownFlag);
            std::vector<AppId> roots = luadata::LibraryAppIds();
            if (roots.empty()) return count;

            std::uint32_t written = 0;
            std::uint32_t advertisedAdds = 0;
            const bool canScanOriginal = appList && count <= size;

            for (AppId appId : roots) {
                bool alreadyInList = false;
                if (canScanOriginal) {
                    for (std::uint32_t i = 0; i < count; ++i) {
                        if (appList[i] == appId) {
                            alreadyInList = true;
                            break;
                        }
                    }
                }
                if (alreadyInList) continue;

                ++advertisedAdds;
                if (appList && count + written < size) {
                    appList[count + written] = appId;
                    ++written;
                }
            }

            std::uint32_t advertisedTotal = count + advertisedAdds;
            if (advertisedTotal < count) advertisedTotal = (std::numeric_limits<std::uint32_t>::max)();
            AC_LOG_INFO_ONCE(kModule, "GetSubscribedApps: original=%u roots=%zu written=%u advertised=%u buffer=%u.",
                count, roots.size(), written, advertisedTotal, size);
            if (written < advertisedAdds && size != 0) {
                // size==0 is Steam's probe call (it only wants the advertised count),
                // so a zero-sized write there is expected, not an error.
                AC_LOG_WARN(kModule, "GetSubscribedApps: caller buffer too small (%u); advertised %u root(s) but wrote %u.",
                    size, advertisedAdds, written);
            }
            return advertisedTotal;
        }

        bool h_SendCallbackToPipe(void* engine, HSteamPipe pipe, HSteamUser user, int cb,
            void* data, int size) {
            // Force the "licenses changed" flag so Steam re-reads ownership.
            if (cb == constants::kCallbackAppLicensesChanged && data) {
                *static_cast<bool*>(data) = true;
                ++g_state.licenseReloadForcedCount;
                // Debounce: SmartIdLog-style — the first call logs immediately, then
                // subsequent identical calls within the burst are suppressed. The full
                // count is visible in status.json as license_reload_forced_count.
                AC_LOG_DEBUG_ONCE(kModule, "SendCallbackToPipe: forced m_bReloadAll on AppLicensesChanged.");
            }
            else if (cb == constants::kCallbackAppLicensesChanged) {
                AC_LOG_WARN(kModule, "SendCallbackToPipe: AppLicensesChanged without reload payload; "
                    "ownership may not refresh.");
            }

            // ── AetherOnline achievement callbacks ──────────────────────────────────────
            // Steam processes the stats store under the REAL app id (see the
            // wire-layer rewrites and the IClientUserStats stats-scope) and delivers
            // the achievement callbacks (1101/1102/1103/1109) with the real app id in
            // m_nGameID. Two consumers need them with different app ids:
            //
            //   * the Steam overlay (unlock toast + Shift+Tab panel) is keyed to the
            //     REAL app id (SteamOverlayGameId, see BuildSpawnEnvBlock) — it
            //     consumes the real copies;
            //   * the game's own Steamworks flow is bound to the masked 480 session
            //     (RequestCurrentStats / StoreStats completion, icon fetch) — it needs
            //     the 480 copies.
            //
            // Delivery policy (single source of truth in NeedsRewrittenGameCopy):
            //   * every callback is first dispatched with the real app id — this is
            //     what the overlay consumes (unlock toast + panel refresh);
            //   * UserStatsReceived (1101), UserStatsStored (1102) and
            //     UserAchievementIconFetched (1109) are ALSO dispatched with the app id
            //     rewritten to 480, so the game's own handlers still fire;
            //   * UserAchievementStored (1103) is dispatched with the real app id ONLY:
            //     it drives the overlay unlock toast, and a second (480) copy would
            //     make the overlay show the same toast twice (the duplicate
            //     notification). The game's achievement state is updated through the
            //     stats store, not via 1103, so no 480 copy is needed.
            //
            // m_nGameID is the first field (uint64) in all achievement callback structs:
            //   UserStatsReceived_t::m_nGameID
            //   UserStatsStored_t::m_nGameID
            //   UserAchievementStored_t::m_nGameID
            //   UserAchievementIconFetched_t::m_nGameID
            const AppId realApp = g_state.aetherOnlineRealAppId.load(std::memory_order_acquire);
            if (realApp != 0 && realApp != constants::kSpacewarAppId &&
                constants::achievement_cb::IsAchievementCallback(cb) &&
                data && size >= static_cast<int>(sizeof(std::uint64_t)))
            {
                auto* pGameId = static_cast<std::uint64_t*>(data);
                if ((*pGameId & constants::kGameIdAppIdMask) == realApp) {
                    // First delivery: real app id (overlay toast/panel consumer).
                    const bool firstOk = o_SendCallbackToPipe(engine, pipe, user, cb, data, size);

                    // Second delivery: 480 copy for the game's own handlers, except
                    // for UserAchievementStored (1103) whose 480 copy would duplicate
                    // the overlay unlock toast (see NeedsRewrittenGameCopy).
                    if (NeedsRewrittenGameCopy(cb)
                        && RewriteAetherOnlineCallbackGameId(cb, data, size)) {
                        return o_SendCallbackToPipe(engine, pipe, user, cb, data, size);
                    }
                    return firstOk;
                }
            }

            return o_SendCallbackToPipe(engine, pipe, user, cb, data, size);
        }

        bool h_LoadPackage(PackageInfo* info, std::uint8_t* sha1, std::int32_t cn, void* p4) {
            bool result = o_LoadPackage(info, sha1, cn, p4);

            if (info && info->packageId == 0) {
                // PR-style guard: only inject when Steam reports the package usable.
                // Injecting into a non-available package gets the vector clobbered when
                // Steam reloads it; defer to DoStartupInjection instead.
                if (info->status != 0) {
                    // Publish the pointer here too, so the deferred DoStartupInjection
                    // can actually run post-login (the package is re-read from g_state).
                    g_state.pPackage0.store(info);
                    AC_LOG_WARN(kModule, "Package 0 status=%u not available; deferring injection.",
                        info->status);
                }
                else {
                    // SeedPackage0 owns the g_state.pPackage0 publication (store + seed).
                    LicenseManager::SeedPackage0(info);
                }
            }
            return result;
        }

    }  // namespace

    void RegisterOwnershipHooks(HMODULE diversion) {
        if (!diversion) {
            AC_LOG_ERROR(kModule, "Diversion module not loaded.");
            return;
        }
        AC_LOG_INFO(kModule, "Registering ownership hooks.");

        g_state.hookManager.TryHook("CheckAppOwnership", "steamclient", diversion,
            o_CheckAppOwnership, h_CheckAppOwnership);
        g_state.hookManager.TryHook("MarkLicenseAsChanged", "steamclient", diversion,
            o_MarkLicenseAsChanged, h_MarkLicenseAsChanged);
        g_state.hookManager.TryHook("GetPackageInfo", "steamclient", diversion,
            o_GetPackageInfo, h_GetPackageInfo);
        g_state.hookManager.TryHook("GetSubscribedApps", "steamclient", diversion,
            o_GetSubscribedApps, h_GetSubscribedApps);
        g_state.hookManager.TryHook("SendCallbackToPipe", "steamclient", diversion,
            o_SendCallbackToPipe, h_SendCallbackToPipe);
        g_state.hookManager.TryHook("LoadPackage", "steamclient", diversion,
            o_LoadPackage, h_LoadPackage);

        // The grow helper and license-update resolver live in LicenseManager.
        LicenseManager::Init(diversion);
    }

    void ShutdownOwnershipHooks() {
        StopUnlockSummary();
    }

}  // namespace ac::hooks
