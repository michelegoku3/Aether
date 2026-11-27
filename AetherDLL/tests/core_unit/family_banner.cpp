#include "cases.h"

#include <cstdint>
#include <vector>

#include "core/FamilyClassification.h"
#include "scripting/LuaData.h"

// The owner banner writes LastOwner, which the family-lender detector reads as
// a real Steam Family licence. The app must stay the one Aether provides: the
// banner changes what the client draws, nothing else. These cases pin the rule
// in both places it is used - the pure classification and the LuaData gate.
void FamilyBannerTest() {
    using namespace ac::familyclass;

    // --- the banner refines the family flag, it does not create one --------
    CHECK(ClassifyFamily(false, false) == FamilyKind::None);
    CHECK(ClassifyFamily(true, false) == FamilyKind::RealShare);
    CHECK(ClassifyFamily(true, true) == FamilyKind::Banner);
    CHECK(ClassifyFamily(false, true) == FamilyKind::None);

    // --- which apps Aether provides ---------------------------------------
    CHECK(!ProvidesApp(false, false, FamilyKind::None)); // no .lua
    CHECK(ProvidesApp(true, false, FamilyKind::None));   // plain spoof
    CHECK(!ProvidesApp(true, true, FamilyKind::None));   // genuinely owned
    CHECK(!ProvidesApp(true, false, FamilyKind::RealShare));
    CHECK(!ProvidesApp(true, true, FamilyKind::RealShare));
    CHECK(ProvidesApp(true, false, FamilyKind::Banner)); // display-only banner
    CHECK(!ProvidesApp(true, true, FamilyKind::Banner));

    // --- is this family bit our own banner stamp? -------------------------
    // Our stamp always names the banner account (AppManifestOwner writes it,
    // FamilyLenders resolves it), so an answer that names no lender is a real
    // share we cannot claim: the app stays Valve's.
    const std::vector<std::uint32_t> listed{1091500, 1868140};
    constexpr std::uint64_t kBannerSteamId = 76561199523209859ull;
    const auto bannerAccount = static_cast<std::uint32_t>(kBannerSteamId);

    CHECK(IsOwnBannerStamp(1868140, bannerAccount, true, listed, kBannerSteamId));
    CHECK(!IsOwnBannerStamp(1868140, 0, true, listed, kBannerSteamId));
    CHECK(!IsOwnBannerStamp(1868140, bannerAccount, false, listed, kBannerSteamId));
    CHECK(!IsOwnBannerStamp(1868140, bannerAccount, true, listed, 0));
    CHECK(!IsOwnBannerStamp(2000, bannerAccount, true, listed, kBannerSteamId)); // not listed
    CHECK(!IsOwnBannerStamp(1868140, 12345, true, listed, kBannerSteamId));      // a real lender

    // --- which family answers are Valve's licence -------------------------
    // Package 0 must not license these: the client would offer PLAY and the CM
    // would stop the session minutes later.
    constexpr std::uint32_t kThisLogin = 1624650063;
    CHECK(!IsValveFamilyLicense(1868140, 0, kThisLogin, true, listed, kBannerSteamId));             // no lender
    CHECK(!IsValveFamilyLicense(1868140, kThisLogin, kThisLogin, true, listed, kBannerSteamId));    // our own
    CHECK(!IsValveFamilyLicense(1868140, bannerAccount, kThisLogin, true, listed, kBannerSteamId)); // our stamp
    CHECK(IsValveFamilyLicense(1868140, 12345, kThisLogin, true, listed, kBannerSteamId));          // another account
    CHECK(IsValveFamilyLicense(1868140, 12345, kThisLogin, false, listed, kBannerSteamId));         // banner off
    CHECK(IsValveFamilyLicense(2000, 12345, kThisLogin, true, listed, kBannerSteamId));             // not listed

    // --- LuaData keeps the display flag and reopens the spoofing gate ------
    constexpr std::uint32_t kApp = 424242;
    ac::luadata::RecordDepot(kApp, "00");
    CHECK(ac::luadata::IsConfigured(kApp));
    CHECK(ac::luadata::HasDepot(kApp)); // configured, not owned, not shared
    CHECK(!ac::luadata::IsFamilyShared(kApp));
    CHECK(!ac::luadata::IsRealFamilyShare(kApp));
    CHECK(ac::luadata::FamilySharedCount() == 0);
    CHECK(ac::luadata::BannerFamilyCount() == 0);

    // A real Steam Family licence: Valve's state is the truth.
    ac::luadata::MarkFamilyShared(kApp, bannerAccount, /*ownBanner=*/false);
    CHECK(ac::luadata::IsFamilyShared(kApp));
    CHECK(ac::luadata::IsRealFamilyShare(kApp));
    CHECK(!ac::luadata::HasDepot(kApp));
    CHECK(ac::luadata::FamilySharedCount() == 1);
    CHECK(ac::luadata::BannerFamilyCount() == 0);

    // Our own banner: still drawn as family, still Aether's app.
    ac::luadata::MarkFamilyShared(kApp, bannerAccount, /*ownBanner=*/true);
    CHECK(ac::luadata::IsFamilyShared(kApp));     // the card keeps the banner
    CHECK(!ac::luadata::IsRealFamilyShare(kApp)); // but it is not a licence
    CHECK(ac::luadata::HasDepot(kApp));           // ...so Aether still provides it
    CHECK(ac::luadata::FamilySharedCount() == 0);
    CHECK(ac::luadata::BannerFamilyCount() == 1);

    // The same app turning into a real share drops the banner bit, even when
    // the stamp used the same lender.
    ac::luadata::MarkFamilyShared(kApp, 12345, /*ownBanner=*/false);
    CHECK(ac::luadata::IsRealFamilyShare(kApp));
    CHECK(!ac::luadata::HasDepot(kApp));
    CHECK(ac::luadata::BannerFamilyCount() == 0);

    // The banner flag never outlives the family one.
    ac::luadata::MarkSpoofed(kApp);
    CHECK(!ac::luadata::IsFamilyShared(kApp));
    CHECK(!ac::luadata::IsRealFamilyShare(kApp));
    CHECK(ac::luadata::BannerFamilyCount() == 0);
    CHECK(ac::luadata::HasDepot(kApp));

    ac::luadata::MarkFamilyShared(kApp, bannerAccount, /*ownBanner=*/true);
    ac::luadata::MarkOwned(kApp);
    CHECK(!ac::luadata::IsFamilyShared(kApp));
    CHECK(!ac::luadata::IsRealFamilyShare(kApp));
    CHECK(ac::luadata::BannerFamilyCount() == 0);
    CHECK(!ac::luadata::HasDepot(kApp)); // genuinely owned: Steam answers

    // --- Map-derived family share reconciliation ----------------------------
    // A share inferred solely from FamilyLenders (`fromSteam=false`) recovers
    // to Spoofed if FamilyLenders drops it, whereas a real Steam-reported share
    // (`fromSteam=true`) stays sticky.
    CHECK(ShouldClearMapFamilyShare(true, false, 0u, false));
    CHECK(!ShouldClearMapFamilyShare(false, false, 0u, false)); // Steam-reported stays
    CHECK(!ShouldClearMapFamilyShare(true, true, 0u, false));   // already banner
    CHECK(!ShouldClearMapFamilyShare(true, false, 12345u, false));
    CHECK(ShouldClearMapFamilyShare(true, false, bannerAccount, true)); // banner enabled for lender

    // Setting banner account == active account is never treated as a valid
    // banner stamp or Valve family license.
    CHECK(!IsOwnBannerStamp(1868140, kThisLogin, true, listed, 76561197960265728ull + kThisLogin, kThisLogin));
    CHECK(!IsValveFamilyLicense(1868140, kThisLogin, kThisLogin, true, listed, 76561197960265728ull + kThisLogin));

    // --- LuaData map-derived family reconciliation ---------------------------
    ac::luadata::MarkFamilyShared(kApp, 12345, /*ownBanner=*/false, /*fromSteam=*/false);
    CHECK(ac::luadata::IsMapDerivedFamilyShare(kApp));
    CHECK(ac::luadata::IsRealFamilyShare(kApp));
    CHECK(ac::luadata::MapDerivedFamilyAppIds() == std::vector<std::uint32_t>{kApp});
    CHECK(ac::luadata::ClearMapFamilyShare(kApp));
    CHECK(!ac::luadata::IsMapDerivedFamilyShare(kApp));
    CHECK(!ac::luadata::IsFamilyShared(kApp));
    CHECK(ac::luadata::HasDepot(kApp));

    // A Steam-confirmed share clears the map-derived flag so it stays sticky.
    ac::luadata::MarkFamilyShared(kApp, 12345, /*ownBanner=*/false, /*fromSteam=*/false);
    ac::luadata::MarkFamilyShared(kApp, 12345, /*ownBanner=*/false, /*fromSteam=*/true);
    CHECK(!ac::luadata::IsMapDerivedFamilyShare(kApp));
    CHECK(!ac::luadata::ClearMapFamilyShare(kApp));
    CHECK(ac::luadata::IsRealFamilyShare(kApp));

    // A later PassThrough refresh where Package 0 masks borrowed/familyShared
    // (fromSteam=false) must NEVER downgrade a Steam-confirmed family share back
    // to map-derived, otherwise ShouldClearMapFamilyShare would wipe it.
    ac::luadata::MarkFamilyShared(kApp, 12345, /*ownBanner=*/false, /*fromSteam=*/false);
    CHECK(!ac::luadata::IsMapDerivedFamilyShare(kApp));
    CHECK(!ac::luadata::ClearMapFamilyShare(kApp));
    CHECK(ac::luadata::IsRealFamilyShare(kApp));
}
