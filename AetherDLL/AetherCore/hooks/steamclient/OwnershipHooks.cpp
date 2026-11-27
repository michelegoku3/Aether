#include "pch.h"
#include "hooks/steamclient/OwnershipHooks.h"

#include <algorithm>
#include <atomic>
#include <chrono>
#include <mutex>
#include <sstream>
#include <string>
#include <unordered_set>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/AppManifestOwner.h"
#include "core/Constants.h"
#include "core/FamilyClassification.h"
#include "core/HookManager.h"
#include "core/Logger.h"
#include "core/OwnershipDecision.h"
#include "core/Settings.h"
#include "core/SteamTypes.h"
#include "core/StructGuard.h"
#include "core/Workers.h"
#include "credentials/CredentialStore.h"
#include "family/FamilyLenders.h"
#include "hooks/aetheronline/PresenceSession.h"
#include "hooks/license/LicenseManager.h"
#include "hooks/steamclient/CallbackCopy.h"
#include "scripting/LuaData.h"
#include "utils/Paths.h"
#include "utils/SmartIdLog.h"

namespace ac::hooks {
namespace {

constexpr const char* kModule = "Ownership";
using namespace ac::steam;

// ---- Hook signatures -------------------------------------------------------
using CheckAppOwnership_t = bool (*)(void*, AppId, AppOwnership*);
using MarkLicenseAsChanged_t = std::int64_t (*)(void*, std::uint32_t, bool);
using ProcessPendingLicenseUpdates_t = bool (*)(void*);
using GetPackageInfo_t = void* (*)(void*, std::uint32_t, std::int64_t);
using SendCallbackToPipe_t = bool (*)(void*, HSteamPipe, HSteamUser, int, void*, int);
using LoadPackage_t = bool (*)(PackageInfo*, std::uint8_t*, std::int32_t, void*);

CheckAppOwnership_t o_CheckAppOwnership = nullptr;
MarkLicenseAsChanged_t o_MarkLicenseAsChanged = nullptr;
ProcessPendingLicenseUpdates_t o_ProcessPendingLicenseUpdates = nullptr;
GetPackageInfo_t o_GetPackageInfo = nullptr;
SendCallbackToPipe_t o_SendCallbackToPipe = nullptr;
LoadPackage_t o_LoadPackage = nullptr;

// Smart debounced once-per-app ownership outcome logs (legit/family are rare;
// the "unlocked" outcome is aggregated per-file instead of per-appid).
logutil::SmartIdLog s_logLegit(kModule, "Legit-owned AppIds");
logutil::SmartIdLog s_logFamily(kModule, "Family-shared AppIds");

// ---------------------------------------------------------------------------
// Unlock summary.
//
// Every spoofed ownership outcome is recorded into a session set. A debounce
// thread then emits ONE line per .lua file once the burst settles:
//   INFO  "Unlocked all AppID for 1144200.lua."          (all expected ok)
//   WARN  "Not unlocked for 1144200.lua: 1144201, ..."   (single message, ids)
//
// Expected ids per file = the ids the file contributes that still need spoofing
// (luadata::HasDepot — configured, not genuinely owned, not a real family share;
// the cosmetic owner banner keeps an app here).
// The debounce re-arms on every unlock, so late checks (game launch, hot-reload)
// produce a fresh summary instead of being missed. The thread only reads
// LuaData (shared locks) and writes logs — it never touches Steam internals.
// ---------------------------------------------------------------------------
std::mutex s_unlockMutex;
std::unordered_set<AppId> s_unlockedAppIds;
// The thread runs in workers::StartWorker; stop+join are centralized.
std::atomic<bool> s_summaryStarted{false};
std::atomic<std::int64_t> s_summaryDeadline{0};
// Last emitted outcome fingerprint: "per-file missing list" as a stable string.
// The summary is re-logged only when this fingerprint changes (or on the very
// first emission), so repeated bursts (login, hot-reload, launch) stay quiet.
std::string s_lastSummaryFingerprint;
bool s_hasEmitted = false;

std::int64_t SteadyNowMs() {
    return std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now().time_since_epoch())
        .count();
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
    struct SummaryRow {
        std::string base;
        std::vector<AppId> missing;
    };
    std::vector<SummaryRow> rows;
    rows.reserve(byFile.size());

    for (const auto& [path, ids] : byFile) {
        std::vector<AppId> expected;
        expected.reserve(ids.size());
        for (AppId id : ids) {
            // Only ids that still require spoofing; owned/family-shared ids are
            // legitimately covered by Steam and must not be reported as missing.
            if (luadata::HasDepot(id))
                expected.push_back(id);
        }
        if (expected.empty()) {
            // Emit the per-file OK line even when the file contributes no
            // currently eligible IDs.
            rows.push_back({paths::BaseName(path), {}});
            continue;
        }
        ++files;
        expectedTotal += expected.size();

        std::vector<AppId> missing;
        for (AppId id : expected) {
            if (unlocked.count(id)) {
                ++unlockedTotal;
            } else {
                missing.push_back(id);
            }
        }

        const std::string base = paths::BaseName(path);
        fingerprint << base << ':';
        if (missing.empty()) {
            fingerprint << "OK;";
        } else {
            // Full missing-id list in the fingerprint so a change in which ids
            // are missing (even with the same count) re-logs the summary.
            for (std::size_t i = 0; i < missing.size(); ++i) {
                if (i)
                    fingerprint << ',';
                fingerprint << missing[i];
            }
            fingerprint << ';';
        }
        rows.push_back({base, std::move(missing)});
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
    for (const auto& row : rows) {
        const auto& missing = row.missing;
        if (missing.empty()) {
            AC_LOG_INFO(kModule, "Unlocked all AppID for %s.", row.base.c_str());
        } else {
            std::ostringstream list;
            for (std::size_t i = 0; i < missing.size(); ++i) {
                if (i)
                    list << ", ";
                list << missing[i];
            }
            AC_LOG_WARN(kModule, "Not unlocked for %s: %s.", row.base.c_str(), list.str().c_str());
        }
    }

    AC_LOG_DEBUG(kModule, "Unlock summary: %zu/%zu appids across %zu file(s).", unlockedTotal, expectedTotal, files);
}

void UnlockSummaryWorker(std::atomic<bool>& stop) {
    for (;;) {
        if (stop.load(std::memory_order_relaxed))
            return;
        const std::int64_t deadline = s_summaryDeadline.load(std::memory_order_relaxed);
        if (deadline != 0 && SteadyNowMs() >= deadline) {
            s_summaryDeadline.store(0, std::memory_order_relaxed);
            LogUnlockedSummary();
        }
        Sleep(constants::kUnlockSummaryTickMs);
    }
}

void ArmUnlockSummary() {
    s_summaryDeadline.store(SteadyNowMs() + constants::kUnlockSummaryDebounceMs, std::memory_order_relaxed);
    bool expected = false;
    if (s_summaryStarted.compare_exchange_strong(expected, true)) {
        if (!workers::StartWorker("unlock_summary", UnlockSummaryWorker)) {
            s_summaryStarted.store(false, std::memory_order_relaxed);
            AC_LOG_ERROR(kModule, "Could not start the unlock summary worker; "
                                  "ownership spoofing summary will not be logged.");
        }
    }
}

void StopUnlockSummary() {
    // stop+join happen inside workers::Shutdown() (called before
    // ShutdownOwnershipHooks in dllmain).
    AC_LOG_DEBUG(kModule, "Unlock summary stop requested; join handled by workers::Shutdown.");
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

// Account id the app-details page should name for a spoofed app. A real family
// lender wins over the live account; the opt-in cosmetic banner replaces both
// for its listed apps. Reading the live account costs a registry lookup, so
// the caller only asks when it is about to spoof.
std::uint32_t ResolveSpoofOwnerAccountId(AppId app, std::uint32_t familyLender) {
    if (const auto settings = Settings::Snapshot();
        settings && settings->libraryOwnerBanner && settings->libraryOwnerBannerSteamId != 0 &&
        std::find(settings->libraryOwnerBannerApps.begin(), settings->libraryOwnerBannerApps.end(), app) !=
            settings->libraryOwnerBannerApps.end()) {
        return static_cast<std::uint32_t>(settings->libraryOwnerBannerSteamId);
    }
    if (familyLender != 0)
        return familyLender;
    return credential::ReadActiveUserId();
}

// True when this app's family answer is the stamp Aether's own owner banner
// wrote, not a licence. The app keeps the family look, but it is still one
// Aether provides: without this, the stamp would turn it into a Steam-provided
// app and switch off every gate keyed on HasDepot. An answer that names no
// lender is a real share, so the manifest's lender (the map FamilyLenders
// builds) is the evidence in that case: our stamp always names one.
bool IsOwnBannerAnswer(AppId app, std::uint32_t lender) {
    const auto settings = Settings::Snapshot();
    if (!settings)
        return false;
    const std::uint32_t evidence = lender != 0 ? lender : ac::family::LenderFor(app);
    return familyclass::IsOwnBannerStamp(app, evidence, settings->libraryOwnerBanner, settings->libraryOwnerBannerApps,
                                         settings->libraryOwnerBannerSteamId, credential::ReadActiveUserId());
}

// True when the family answer for this app is a licence Valve granted to another
// account, not Aether's banner stamp. Nothing Aether answers may overrule it.
bool IsValveFamilyAnswer(AppId app, std::uint32_t lender) {
    if (lender == 0 || IsOwnBannerAnswer(app, lender))
        return false;
    return lender != credential::ReadActiveUserId();
}

// ---- Hook bodies -----------------------------------------------------------

// Both CUser::MarkLicenseAsChanged and CUser::CheckAppOwnership receive the
// same CUser* instance as `self`. Capturing from CheckAppOwnership too recovers
// when hooks install after Steam's startup MarkLicenseAsChanged already fired.
void CaptureCUser(void* self, const char* source) {
    if (!self)
        return;
    void* prev = nullptr;
    if (g_state.pCUser.compare_exchange_strong(prev, self)) {
        AC_LOG_INFO(kModule, "Captured CUser pointer 0x%p (%s).", self, source);
        // First capture is post-login: Steam may have logged into a different
        // account since startup, so re-apply the owner policy (the "From the
        // library of <account>" banner reads it) and refresh the family lender map.
        ac::appmanifest::RequestOwnerApply();
        ac::family::RequestRefresh();
        LicenseManager::OnUserCaptured();
    }
}

bool CheckAppOwnershipImpl(void* self, AppId app, AppOwnership* out) {
    bool result = o_CheckAppOwnership(self, app, out);
    CaptureCUser(self, "CheckAppOwnership");
    // A .lua app needs Aether until Steam provides it. Genuinely owned apps
    // pass through; a family-shared app still needs the banner override below,
    // because Steam's answer must keep naming the lender.
    if (!out || !luadata::IsConfigured(app) || luadata::IsOwned(app))
        return result;

    // Struct guard. This is the write that does NOT crash when the
    // layout is wrong: we would simply feed Steam confident nonsense
    // about ownership. The 18-bool block (+0x24..+0x35, proven
    // identical on both shipped builds) is the cheap tell — Valve
    // only ever stores 0 or 1 there, so a shifted layout fails the
    // test on the first byte that holds a pointer fragment.
    if (const auto reason = abi::guard::CheckOwnership(out); !abi::guard::Passed(reason)) {
        abi::guard::CountRejection("AppOwnership", reason);
        return result; // hand back Steam's own answer, untouched
    }

    // Match LumaCore's conservative classification: a Lua app is considered
    // Steam-provided only when Steam reports a released app present in more than
    // one package. This avoids the over-broad "if (result)" path that can make
    // every injected Lua app disappear, and it also avoids treating Steam common
    // redistributables/package-tool entries as user-owned games just because
    // they have a non-zero package id.
    const bool spoofed = luadata::IsSpoofed(app);
    std::uint32_t familyLender = luadata::FamilyLender(app);
    const std::uint32_t mapLender = ac::family::LenderFor(app);
    const std::uint32_t activeAccountId = credential::ReadActiveUserId();
    const bool steamReportedFamily = result && out->existInPackageNums > 1 &&
                                     out->releaseState == AppReleaseState::Released &&
                                     (out->borrowed || out->familyShared);
    if (!steamReportedFamily &&
        familyclass::ShouldClearMapFamilyShare(luadata::IsMapDerivedFamilyShare(app),
                                               luadata::IsFamilyShared(app) && !luadata::IsRealFamilyShare(app),
                                               mapLender, IsOwnBannerAnswer(app, mapLender)) &&
        luadata::ClearMapFamilyShare(app)) {
        AC_LOG_INFO(
            kModule,
            "CheckAppOwnership: cleared stale map-derived family share for app %u (old_lender=%u, new_lender=%u).", app,
            familyLender, mapLender);
        familyLender = 0;
        LicenseManager::RequestAppInPackage0(app);
        LicenseManager::RequestLicenseRefresh();
    }
    // Steam's CheckAppOwnership is not the family-sharing source: an installed
    // family game can answer "not provided" while the SteamUI still lists a
    // real lender. Fill the gap from the client's FamilyGroups data. A real
    // Steam family answer with a named lender wins; the map only fills a
    // missing lender. The licence stays Valve's: nothing below adds the app to
    // package 0.
    if (familyLender == 0 && mapLender != 0 && mapLender != activeAccountId) {
        const bool rawFamilyWithLender =
            steamReportedFamily && out->steamId32 != 0 && out->steamId32 != activeAccountId;
        const bool ownBanner = IsOwnBannerAnswer(app, mapLender);
        const bool directLicense = !ownBanner && ownership::IsDirectlyLicensedByActiveAccount(
                                                     result, out->existInPackageNums, out->releaseState, out->packageId,
                                                     out->borrowed, out->familyShared, out->freeLicense);
        if (directLicense) {
            AC_LOG_DEBUG_ONCE(
                kModule,
                "CheckAppOwnership: ignoring map lender %u for app %u (Steam reports direct license: pkg=%u, exist=%u, "
                "free=%d).",
                mapLender, app, out->packageId, out->existInPackageNums, out->freeLicense ? 1 : 0);
        } else if (!rawFamilyWithLender) {
            const bool wasFamilyShared = luadata::IsFamilyShared(app);
            luadata::MarkFamilyShared(app, mapLender, ownBanner, /*fromSteam=*/false);
            familyLender = mapLender;
            if (!wasFamilyShared) {
                AC_LOG_INFO(
                    kModule,
                    "CheckAppOwnership: app %u adopted map-derived family share (lender=%u, own_banner=%d, pkg=%u, "
                    "exist=%u, free=%d).",
                    app, mapLender, ownBanner ? 1 : 0, out->packageId, out->existInPackageNums,
                    out->freeLicense ? 1 : 0);
                ac::appmanifest::RequestOwnerApply();
            }
            if (!ownBanner)
                LicenseManager::RequestRemoveAppFromPackage0(app);
            LicenseManager::RequestLicenseRefresh();
            RecordOwnershipOutcome(app, ownBanner ? OwnershipOutcome::Unlocked : OwnershipOutcome::FamilyShared);
        }
    }
    ownership::OwnershipFacts facts{
        result,
        out->existInPackageNums,
        out->releaseState,
        out->borrowed,
        out->familyShared,
        true, // IsConfigured was checked above
        0,
        spoofed,
        familyLender,
        IsValveFamilyAnswer(app, familyLender),
        out->packageId == 0 && out->freeLicense,
        out->packageId == 0 && !out->freeLicense,
    };
    // Resolving the account reads the registry; a family lender already names
    // the banner owner, and a pass-through needs no owner at all.
    if (familyLender != 0 || spoofed || !ownership::IsSteamProvided(facts))
        facts.spoofOwnerAccountId = ResolveSpoofOwnerAccountId(app, familyLender);
    const ownership::OwnershipDecision decision = ownership::DecideOwnership(facts);

    if (decision.action == ownership::OwnershipAction::PassThrough) {
        if (decision.steamFamilyShared) {
            // A boot-time owner pass ran before Steam could classify the app,
            // so the manifest may carry Aether's stamp. A real family license
            // needs Steam's own value back to name a local owner; when Package 0
            // temporarily masks borrowed/familyShared and substitutes the active
            // account id, preserve both the confirmed lender and sticky state.
            const std::uint32_t lender = ownership::ResolveObservedFamilyLender(steamReportedFamily, out->steamId32,
                                                                                familyLender, activeAccountId);
            const bool ownBanner = IsOwnBannerAnswer(app, lender);
            const bool newlyFamilyShared = !luadata::IsFamilyShared(app);
            const bool stickyFromSteam =
                steamReportedFamily || (familyLender != 0 && !luadata::IsMapDerivedFamilyShare(app));
            const bool stateChanged = newlyFamilyShared || lender != familyLender ||
                                      (steamReportedFamily && luadata::IsMapDerivedFamilyShare(app));
            if (stateChanged) {
                luadata::MarkFamilyShared(app, lender, ownBanner, stickyFromSteam);
                AC_LOG_INFO(kModule,
                            "CheckAppOwnership: app %u %s family share (lender=%u, prev_lender=%u, from_steam=%d, "
                            "own_banner=%d, pkg=%u, exist=%u, raw_steamId32=%u).",
                            app, newlyFamilyShared ? "recorded" : "updated", lender, familyLender,
                            stickyFromSteam ? 1 : 0, ownBanner ? 1 : 0, out->packageId, out->existInPackageNums,
                            out->steamId32);
            }
            if (newlyFamilyShared) {
                ac::appmanifest::RequestOwnerApply();
            }
            if (!ownBanner && (stateChanged || out->packageId == 0)) {
                LicenseManager::RequestRemoveAppFromPackage0(app);
            }
            RecordOwnershipOutcome(app, ownBanner ? OwnershipOutcome::Unlocked : OwnershipOutcome::FamilyShared);
        } else if (decision.needsPackage0Unmask) {
            AC_LOG_INFO(kModule,
                        "CheckAppOwnership: app %u answer masked by package 0 (pkg=%u, exist=%u, free=%d, "
                        "raw_steamId32=%u); deferring classification until root is pruned.",
                        app, out->packageId, out->existInPackageNums, out->freeLicense ? 1 : 0, out->steamId32);
            LicenseManager::RequestRemoveAppFromPackage0(app);
        } else {
            AC_LOG_INFO(kModule, "CheckAppOwnership: app %u classified as owned (pkg=%u, exist=%u, free=%d).", app,
                        out->packageId, out->existInPackageNums, out->freeLicense ? 1 : 0);
            luadata::MarkOwned(app);
            LicenseManager::RequestRemoveAppFromPackage0(app);
            RecordOwnershipOutcome(app, OwnershipOutcome::LegitOwned);
        }
        AC_LOG_DEBUG_ONCE(
            kModule,
            "CheckAppOwnership: %u steam-provided class=%s result=%d exist=%u pkg=%u free=%d release=%u borrowed=%d "
            "familyShared=%d.",
            app, decision.steamFamilyShared ? "family-shared" : (decision.needsPackage0Unmask ? "unmasking" : "owned"),
            result ? 1 : 0, out->existInPackageNums, out->packageId, out->freeLicense ? 1 : 0,
            static_cast<unsigned>(out->releaseState), out->borrowed ? 1 : 0, out->familyShared ? 1 : 0);
        return result;
    }

    // Spoof. The first classification marks the app and adds it to package 0;
    // later calls see Steam report owned because of that entry and keep the
    // spoof through the sticky flag. A recorded family lender is Valve's and
    // never reaches this point (its decision is PassThrough).
    if (!spoofed && familyLender == 0) {
        AC_LOG_INFO(kModule,
                    "CheckAppOwnership: app %u classified as spoofed -> queued package 0 (pkg=%u, exist=%u, free=%d, "
                    "borrowed=%d, familyShared=%d).",
                    app, out->packageId, out->existInPackageNums, out->freeLicense ? 1 : 0, out->borrowed ? 1 : 0,
                    out->familyShared ? 1 : 0);
        luadata::MarkSpoofed(app);
        LicenseManager::RequestAppInPackage0(app);
    }
    out->packageId = 0;
    out->releaseState = AppReleaseState::Released;
    out->freeLicense = false;
    out->ownsLicense = true;
    // We are claiming this app as owned through package 0; the family bit is
    // Aether's own cosmetic banner, never a real share. A lua-only spoof clears
    // both bits.
    out->borrowed = false;
    out->familyShared = decision.familyBanner;
    // The app-details page derives strOwnerSteamID from this account id. Left
    // at 0, a spoofed app looks like a shared app with an unknown owner and
    // Steam renders the generic family banner. A banner app carries its stamp
    // account here so the banner survives the synthetic package-0 license.
    if (decision.ownerAccountId != 0)
        out->steamId32 = decision.ownerAccountId;
    RecordOwnershipOutcome(app, OwnershipOutcome::Unlocked);
    return true;
}

bool h_CheckAppOwnership(void* self, AppId app, AppOwnership* out) {
    bool result = false;
    {
        LicenseManager::SteamLicenseCallScope guard;
        result = CheckAppOwnershipImpl(self, app, out);
    }
    LicenseManager::TryDrainOnSteamThread();
    return result;
}

std::int64_t h_MarkLicenseAsChanged(void* self, std::uint32_t packageId, bool reloadAll) {
    LicenseManager::RecordSteamMainThread("MarkLicenseAsChanged");
    // Atomic store: the captured pointer is read from other threads (IPC hooks).
    // LicenseManager reads it straight from g_state (single source of truth).
    CaptureCUser(self, "MarkLicenseAsChanged");
    std::int64_t result = 0;
    {
        LicenseManager::SteamLicenseCallScope guard;
        result = o_MarkLicenseAsChanged(self, packageId, reloadAll);
    }
    LicenseManager::TryDrainOnSteamThread();
    return result;
}

bool h_ProcessPendingLicenseUpdates(void* cUser) {
    LicenseManager::RecordSteamMainThread("ProcessPendingLicenseUpdates");
    if (cUser)
        CaptureCUser(cUser, "ProcessPendingLicenseUpdates");
    bool result = false;
    {
        LicenseManager::SteamLicenseCallScope guard;
        result = o_ProcessPendingLicenseUpdates(cUser);
    }
    if (result) {
        LicenseManager::OnSteamLicenseUpdatesApplied();
    }
    LicenseManager::TryDrainOnSteamThread();
    return result;
}

void* h_GetPackageInfo(void* self, std::uint32_t packageId, std::int64_t p3) {
    // Atomic compare-and-swap: capture the pointer exactly once.
    void* prev = nullptr;
    if (g_state.pCPackageInfo.compare_exchange_strong(prev, self)) {
        AC_LOG_INFO(kModule, "Captured CPackageInfo pointer 0x%p.", self);
        LicenseManager::WakeStartupRetry();
    }
    void* result = nullptr;
    {
        LicenseManager::SteamLicenseCallScope guard;
        result = o_GetPackageInfo(self, packageId, p3);
    }
    if (g_state.pCPackageInfo.load(std::memory_order_acquire) != nullptr &&
        g_state.pPackage0.load(std::memory_order_acquire) == nullptr) {
        LicenseManager::RequestAcquirePackage0();
    }
    LicenseManager::TryDrainOnSteamThread();
    return result;
}

bool h_SendCallbackToPipe(void* engine, HSteamPipe pipe, HSteamUser user, int cb, void* data, int size) {
    // Force the "licenses changed" flag so Steam re-reads ownership.
    if (cb == constants::kCallbackAppLicensesChanged && data) {
        *static_cast<bool*>(data) = true;
        ++g_state.licenseReloadForcedCount;
        // Debounce: SmartIdLog-style — the first call logs immediately, then
        // subsequent identical calls within the burst are suppressed. The full
        // count is visible in status.json as license_reload_forced_count.
        AC_LOG_DEBUG_ONCE(kModule, "SendCallbackToPipe: forced m_bReloadAll on AppLicensesChanged.");
    } else if (cb == constants::kCallbackAppLicensesChanged) {
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
    // Delivery policy (single source of truth in CallbackCopy):
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
    const AppId realApp = presence::RealAppId();
    if (realApp != 0 && realApp != constants::kSpacewarAppId && constants::achievement_cb::IsAchievementCallback(cb) &&
        data && size >= static_cast<int>(sizeof(std::uint64_t))) {
        auto* pGameId = static_cast<std::uint64_t*>(data);
        if ((*pGameId & constants::kGameIdAppIdMask) == realApp) {
            // First delivery: real app id (overlay toast/panel consumer).
            const bool firstOk = o_SendCallbackToPipe(engine, pipe, user, cb, data, size);

            // Second delivery: a PRIVATE 480 copy. Steam may keep the pointer
            // it was given and deliver it later on another thread, so the
            // caller's bytes must stay the real-app-id payload.
            if (auto masked = callbackcopy::ForMaskedGame(cb, data, size, realApp)) {
                return o_SendCallbackToPipe(engine, pipe, user, cb, masked->data(), size);
            }
            return firstOk;
        }
    }

    return o_SendCallbackToPipe(engine, pipe, user, cb, data, size);
}

bool h_LoadPackage(PackageInfo* info, std::uint8_t* sha1, std::int32_t cn, void* p4) {
    bool result = false;
    {
        LicenseManager::SteamLicenseCallScope guard;
        result = o_LoadPackage(info, sha1, cn, p4);
    }

    if (info && info->packageId == 0) {
        LicenseManager::RecordSteamMainThread("LoadPackage");
        if (info->status == steam::kPackageStatusUnloaded)
            AC_LOG_WARN(kModule, "Package 0 status=%u (key absent); publishing it for on-demand use.", info->status);
        LicenseManager::RequestSeedPackage0(info);
    }
    LicenseManager::TryDrainOnSteamThread();
    return result;
}

} // namespace

void RegisterOwnershipHooks(HMODULE steamclient) {
    if (!steamclient) {
        AC_LOG_ERROR(kModule, "Steamclient module not loaded.");
        return;
    }
    AC_LOG_INFO(kModule, "Registering ownership hooks.");

    g_state.hookManager.TryHook("CheckAppOwnership", "steamclient", steamclient, o_CheckAppOwnership,
                                h_CheckAppOwnership);
    g_state.hookManager.TryHook("MarkLicenseAsChanged", "steamclient", steamclient, o_MarkLicenseAsChanged,
                                h_MarkLicenseAsChanged);
    g_state.hookManager.TryHook("ProcessPendingLicenseUpdates", "steamclient", steamclient,
                                o_ProcessPendingLicenseUpdates, h_ProcessPendingLicenseUpdates);
    g_state.hookManager.TryHook("GetPackageInfo", "steamclient", steamclient, o_GetPackageInfo, h_GetPackageInfo);
    g_state.hookManager.TryHook("SendCallbackToPipe", "steamclient", steamclient, o_SendCallbackToPipe,
                                h_SendCallbackToPipe);
    g_state.hookManager.TryHook("LoadPackage", "steamclient", steamclient, o_LoadPackage, h_LoadPackage);

    // The grow helper and startup monitor live in LicenseManager.
    LicenseManager::Init(steamclient);
    // Starts the family-lender refresh worker; it waits for login and retries.
    ac::family::Init();
}

void ShutdownOwnershipHooks() {
    StopUnlockSummary();
    ac::family::Shutdown();
}

PackageInfo* AcquirePackage0OnSteamThread() {
    if (auto* existing = static_cast<PackageInfo*>(g_state.pPackage0.load(std::memory_order_acquire)))
        return existing;

    auto* getPackageInfo = o_GetPackageInfo;
    void* manager = g_state.pCPackageInfo.load(std::memory_order_acquire);
    if (!getPackageInfo || !manager)
        return nullptr;

    auto* info = static_cast<PackageInfo*>(getPackageInfo(manager, 0, 0));
    if (!info) {
        AC_LOG_INFO(kModule, "AcquirePackage0: GetPackageInfo returned null; package 0 not loaded yet.");
        return nullptr;
    }

    if (info->status == steam::kPackageStatusUnloaded) {
        AC_LOG_WARN(kModule, "AcquirePackage0: package 0 status=%u (key absent); publishing it for on-demand use.",
                    info->status);
    } else {
        AC_LOG_INFO(
            kModule,
            "AcquirePackage0: package 0 acquired via GetPackageInfo on steamclient main thread (missed one-shot "
            "LoadPackage).");
    }
    return info;
}

bool MarkLicenseAsChangedOnSteamThread(void* cUser, std::uint32_t packageId, bool reloadAll) {
    if (!cUser || !o_MarkLicenseAsChanged)
        return false;
    o_MarkLicenseAsChanged(cUser, packageId, reloadAll);
    return true;
}

bool ProcessPendingLicenseUpdatesOnSteamThread(void* cUser) {
    if (!cUser || !o_ProcessPendingLicenseUpdates)
        return false;
    return o_ProcessPendingLicenseUpdates(cUser);
}

bool HasLicenseRefreshHelpers() noexcept {
    return o_MarkLicenseAsChanged != nullptr && o_ProcessPendingLicenseUpdates != nullptr;
}

} // namespace ac::hooks
