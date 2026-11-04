#pragma once

#include <algorithm>
#include <cstdint>
#include <vector>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Family classification: what a family flag means for an app.
//
// Three states, because two different facts share one flag:
//   * a real Steam Family licence - Valve owns this account's achievement and
//     cloud state, so Aether keeps its hands off;
//   * Aether's own owner banner ([library] owner_banner_apps) - the app is
//     still one Aether provides, the flag only makes the client draw it as
//     shared.
//
// `luadata::IsFamilyShared` reports the display fact (both states), and
// `luadata::IsRealFamilyShare` the licence fact (the first one only). Every
// behavioural gate keys on the licence fact; every visual one on the display
// fact.
// ---------------------------------------------------------------------------
namespace ac::familyclass {

enum class FamilyKind {
    None,
    RealShare, // Steam Family licence: Valve answers for the app
    Banner     // Aether's cosmetic owner banner: Aether answers for the app
};

// The banner flag refines the family flag, it never creates one.
inline FamilyKind ClassifyFamily(bool familyShared, bool bannerFamily) {
    if (!familyShared)
        return FamilyKind::None;
    return bannerFamily ? FamilyKind::Banner : FamilyKind::RealShare;
}

// The single "Aether provides this app" rule behind luadata::HasDepot: a script
// configures it, the account does not genuinely own it, and it is not a real
// family share. A banner app is provided, the banner being display-only.
inline bool ProvidesApp(bool configured, bool owned, FamilyKind kind) {
    return configured && !owned && kind != FamilyKind::RealShare;
}

// True when the family answer for `appId` is the stamp AppManifestOwner wrote
// for the cosmetic banner, not a licence: the app is listed, the banner is on,
// and the named lender is the configured banner account. Our stamp always names
// that account, so an answer that names nobody is a real share Aether cannot
// claim (a Steam Family answer may arrive without a lender).
//
// The comparison uses the account configured now: a stamp left by a previously
// configured banner account reads as a foreign lender, so the app stays a real
// share (sticky, like any other observed share) until Steam is restarted.
inline bool IsOwnBannerStamp(steam::AppId appId, std::uint32_t lenderAccountId, bool bannerEnabled,
                             const std::vector<std::uint32_t>& listedApps, std::uint64_t bannerSteamId) {
    if (!bannerEnabled || bannerSteamId == 0 || lenderAccountId == 0)
        return false;
    if (std::find(listedApps.begin(), listedApps.end(), appId) == listedApps.end())
        return false;
    return lenderAccountId == static_cast<std::uint32_t>(bannerSteamId);
}

} // namespace ac::familyclass
