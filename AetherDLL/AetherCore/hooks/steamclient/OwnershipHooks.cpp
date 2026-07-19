#include "pch.h"
#include "hooks/steamclient/OwnershipHooks.h"

#include <limits>
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
using MarkLicenseAsChanged_t = std::int64_t (*)(void*, std::uint32_t, bool);
using GetPackageInfo_t = void* (*)(void*, std::uint32_t, std::int64_t);
using GetSubscribedApps_t = std::uint32_t (*)(void*, AppId*, std::uint32_t, std::uint8_t);
using SendCallbackToPipe_t = bool (*)(void*, HSteamPipe, HSteamUser, int, void*, int);
using LoadPackage_t = bool (*)(PackageInfo*, std::uint8_t*, std::int32_t, void*);

CheckAppOwnership_t o_CheckAppOwnership = nullptr;
MarkLicenseAsChanged_t o_MarkLicenseAsChanged = nullptr;
GetPackageInfo_t o_GetPackageInfo = nullptr;
GetSubscribedApps_t o_GetSubscribedApps = nullptr;
SendCallbackToPipe_t o_SendCallbackToPipe = nullptr;
LoadPackage_t o_LoadPackage = nullptr;

// Smart debounced once-per-app ownership outcome logs
logutil::SmartIdLog s_logLegit(kModule, "Legit-owned AppIds");
logutil::SmartIdLog s_logFamily(kModule, "Family-shared AppIds");
logutil::SmartIdLog s_logUnlocked(kModule, "Unlocked AppIds");

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
            s_logUnlocked.Record(app);
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
        } else {
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
    return advertisedTotal;
}

bool h_SendCallbackToPipe(void* engine, HSteamPipe pipe, HSteamUser user, int cb,
                          void* data, int size) {
    // Force the "licenses changed" flag so Steam re-reads ownership.
    if (cb == constants::kCallbackAppLicensesChanged && data) {
        *static_cast<bool*>(data) = true;
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
        } else {
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

}  // namespace ac::hooks
