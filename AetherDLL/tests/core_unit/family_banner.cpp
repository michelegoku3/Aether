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
    CHECK(ProvidesApp(true, false, FamilyKind::Banner)); // display-only banner

    // --- is this family bit our own banner stamp? -------------------------
    const std::vector<std::uint32_t> listed{1091500, 1868140};
    constexpr std::uint64_t kBannerSteamId = 76561199523209859ull;
    const auto bannerAccount = static_cast<std::uint32_t>(kBannerSteamId);

    CHECK(IsOwnBannerStamp(1868140, bannerAccount, true, listed, kBannerSteamId));
    CHECK(IsOwnBannerStamp(1868140, 0, true, listed, kBannerSteamId)); // answer named no lender
    CHECK(!IsOwnBannerStamp(1868140, bannerAccount, false, listed, kBannerSteamId));
    CHECK(!IsOwnBannerStamp(1868140, bannerAccount, true, listed, 0));
    CHECK(!IsOwnBannerStamp(2000, bannerAccount, true, listed, kBannerSteamId)); // not listed
    CHECK(!IsOwnBannerStamp(1868140, 12345, true, listed, kBannerSteamId));      // a real lender

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
    CHECK(ac::luadata::IsFamilyShared(kApp));      // the card keeps the banner
    CHECK(!ac::luadata::IsRealFamilyShare(kApp));  // but it is not a licence
    CHECK(ac::luadata::HasDepot(kApp));            // ...so Aether still provides it
    CHECK(ac::luadata::FamilySharedCount() == 0);
    CHECK(ac::luadata::BannerFamilyCount() == 1);

    // The banner flag never outlives the family one.
    ac::luadata::MarkSpoofed(kApp);
    CHECK(!ac::luadata::IsFamilyShared(kApp));
    CHECK(ac::luadata::BannerFamilyCount() == 0);
    CHECK(ac::luadata::HasDepot(kApp));

    ac::luadata::MarkFamilyShared(kApp, bannerAccount, /*ownBanner=*/true);
    ac::luadata::MarkOwned(kApp);
    CHECK(!ac::luadata::IsFamilyShared(kApp));
    CHECK(ac::luadata::BannerFamilyCount() == 0);
    CHECK(!ac::luadata::HasDepot(kApp)); // genuinely owned: Steam answers
}
