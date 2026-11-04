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
// and the lender is either unnamed or the configured banner account. A real
// lender (someone else) is a real share even for a listed app.
inline bool IsOwnBannerStamp(steam::AppId appId, std::uint32_t lenderAccountId, bool bannerEnabled,
                             const std::vector<std::uint32_t>& listedApps, std::uint64_t bannerSteamId) {
    if (!bannerEnabled || bannerSteamId == 0)
        return false;
    if (std::find(listedApps.begin(), listedApps.end(), appId) == listedApps.end())
        return false;
    return lenderAccountId == 0 || lenderAccountId == static_cast<std::uint32_t>(bannerSteamId);
}

} // namespace ac::familyclass
