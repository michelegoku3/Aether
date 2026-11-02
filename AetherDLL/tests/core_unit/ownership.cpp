#include "cases.h"

#include <algorithm>

#include "core/OwnershipDecision.h"
#include "scripting/LuaData.h"

namespace {

// Package 0 is the only reason Steam lists a .lua app, and the ownership answer
// is what Aether classifies. These cases pin the seam between the two.

// A freshly published app is unclassified: Steam never asked about it because
// it does not list it. The seed must carry it anyway, or it stays invisible
// forever (the 1.0.0 egg-and-chicken).
void SeedIncludesUnclassifiedLuaApp() {
    const char* path = "C:/fixtures/2369390.lua";
    ac::luadata::BeginFile(path);
    ac::luadata::RecordDepot(2369390, "");
    ac::luadata::RecordLibraryApp(2369390);
    ac::luadata::EndFile();
    CHECK(!ac::luadata::IsOwned(2369390));
    CHECK(!ac::luadata::IsSpoofed(2369390));
    const auto ids = ac::luadata::SeedPackage0Ids();
    CHECK(std::find(ids.begin(), ids.end(), 2369390u) != ids.end());
    // The Lua store is global to this test binary: unload the fixture so the
    // recorded depot cannot leak into another suite's assertions.
    ac::luadata::UnloadFile(path);
}

// Pre-seeded, Steam's first answer arrives with existInPackageNums == 1: it
// must stay a spoof, not become "owned".
void PreSeededLuaAppStaysSpoofed() {
    ac::ownership::OwnershipFacts f{};
    f.steamResult = true;
    f.existInPackageNums = 1;
    f.releaseState = ac::steam::AppReleaseState::Released;
    f.hasDepot = true;
    f.spoofOwnerAccountId = 1234;
    const auto d = ac::ownership::DecideOwnership(f);
    CHECK(d.action == ac::ownership::OwnershipAction::Spoof);
    CHECK_EQ(d.ownerAccountId, 1234u);
}

// A genuinely owned app stays owned: its real license plus the package-0 row
// brings the count to 2.
void PreSeededOwnedAppIsMarkedOwned() {
    ac::ownership::OwnershipFacts f{};
    f.steamResult = true;
    f.existInPackageNums = 2;
    f.releaseState = ac::steam::AppReleaseState::Released;
    f.hasDepot = true;
    const auto d = ac::ownership::DecideOwnership(f);
    CHECK(d.action == ac::ownership::OwnershipAction::PassThrough);
    CHECK(d.steamOwned);
}

// A pre-seeded family app must not lose the lender/banner.
void PreSeededFamilyAppKeepsBanner() {
    ac::ownership::OwnershipFacts f{};
    f.steamResult = true;
    f.existInPackageNums = 2;
    f.releaseState = ac::steam::AppReleaseState::Released;
    f.hasDepot = true;
    f.familyLenderAccountId = 4242;
    f.spoofOwnerAccountId = 4242;
    const auto d = ac::ownership::DecideOwnership(f);
    CHECK(d.action == ac::ownership::OwnershipAction::Spoof);
    CHECK(d.familyBanner);
}

// The status snapshot reports how many managed apps are actually classified.
// Configured ids, spoofed ids, genuinely owned ids and family-shared ids are
// four different populations: a single counter would hide the case where an app
// is configured but Steam never asked about it (so it is not classified yet).
void MembershipCountsTrackClassification() {
    const std::size_t owned0 = ac::luadata::OwnedCount();
    const std::size_t spoofed0 = ac::luadata::SpoofedCount();
    const std::size_t family0 = ac::luadata::FamilySharedCount();
    const std::size_t roots0 = ac::luadata::LibraryAppCount();

    const char* path = "C:/fixtures/424242.lua";
    ac::luadata::BeginFile(path);
    ac::luadata::RecordDepot(424242, "");
    ac::luadata::RecordDepot(424243, "");
    ac::luadata::RecordDepot(424244, "");
    // 424242 is also the numeric root of the file: only roots are library
    // entries, so the depot-only ids must not raise the root count.
    ac::luadata::RecordLibraryApp(424242);
    ac::luadata::EndFile();

    // Merely being configured classifies nothing.
    CHECK_EQ(ac::luadata::OwnedCount(), owned0);
    CHECK_EQ(ac::luadata::SpoofedCount(), spoofed0);
    CHECK_EQ(ac::luadata::FamilySharedCount(), family0);
    CHECK_EQ(ac::luadata::LibraryAppCount(), roots0 + 1);

    ac::luadata::MarkOwned(424242);
    ac::luadata::MarkSpoofed(424243);
    ac::luadata::MarkFamilyShared(424244, 777);
    CHECK_EQ(ac::luadata::OwnedCount(), owned0 + 1);
    CHECK_EQ(ac::luadata::SpoofedCount(), spoofed0 + 1);
    CHECK_EQ(ac::luadata::FamilySharedCount(), family0 + 1);

    // The flags are mutually exclusive per app: reclassifying moves the app
    // between populations instead of inflating two counters.
    ac::luadata::MarkSpoofed(424242);
    CHECK_EQ(ac::luadata::OwnedCount(), owned0);
    CHECK_EQ(ac::luadata::SpoofedCount(), spoofed0 + 2);

    // The Lua store is global to this test binary: unload the fixture so the
    // recorded ids cannot leak into another suite's assertions.
    ac::luadata::UnloadFile(path);
    CHECK_EQ(ac::luadata::OwnedCount(), owned0);
    CHECK_EQ(ac::luadata::SpoofedCount(), spoofed0);
    CHECK_EQ(ac::luadata::FamilySharedCount(), family0);
    CHECK_EQ(ac::luadata::LibraryAppCount(), roots0);
}

// Two different populations feed two different requests. Package 0 must carry
// every configured id (a depot id is what gives the app its content), but the
// PICS request must ask only for library roots: a depot id has no product info
// and would burn one of the one-shot per-app requests.
void LibraryRootsExcludeDepots() {
    const char* path = "C:/fixtures/515151.lua";
    ac::luadata::BeginFile(path);
    ac::luadata::RecordDepot(515151, "");
    ac::luadata::RecordLibraryApp(515151);
    ac::luadata::RecordDepot(515152, std::string(64, 'a'));
    ac::luadata::EndFile();

    const auto roots = ac::luadata::LibraryAppIds();
    CHECK(std::find(roots.begin(), roots.end(), 515151u) != roots.end());
    CHECK(std::find(roots.begin(), roots.end(), 515152u) == roots.end());

    const auto seed = ac::luadata::SeedPackage0Ids();
    CHECK(std::find(seed.begin(), seed.end(), 515151u) != seed.end());
    CHECK(std::find(seed.begin(), seed.end(), 515152u) != seed.end());

    ac::luadata::UnloadFile(path);
}

} // namespace

void OwnershipDecisionTest() {
    using ac::ownership::DecideOwnership;
    using ac::ownership::OwnershipAction;
    using ac::ownership::OwnershipFacts;
    using ac::steam::AppReleaseState;

    auto facts = [](bool result, std::uint32_t exist, AppReleaseState release) {
        OwnershipFacts f;
        f.steamResult = result;
        f.existInPackageNums = exist;
        f.releaseState = release;
        f.hasDepot = true;
        f.spoofOwnerAccountId = 222;
        return f;
    };

    {
        // A released app in more than one package, not borrowed: Steam owns it.
        OwnershipFacts f = facts(true, 2, AppReleaseState::Released);
        const auto d = DecideOwnership(f);
        CHECK(d.action == OwnershipAction::PassThrough);
        CHECK(d.steamOwned);
        CHECK(!d.steamFamilyShared);
        CHECK_EQ(d.ownerAccountId, 0u);
    }
    {
        // A real family license: pass through, mark family, keep the lender.
        OwnershipFacts f = facts(true, 2, AppReleaseState::Released);
        f.borrowed = true;
        const auto d = DecideOwnership(f);
        CHECK(d.action == OwnershipAction::PassThrough);
        CHECK(!d.steamOwned);
        CHECK(d.steamFamilyShared);
    }
    {
        // The familyShared flag alone is enough, even if borrowed is clear.
        OwnershipFacts f = facts(true, 2, AppReleaseState::Released);
        f.familyShared = true;
        const auto d = DecideOwnership(f);
        CHECK(d.action == OwnershipAction::PassThrough);
        CHECK(d.steamFamilyShared);
        CHECK(!d.steamOwned);
    }
    {
        // Steam does not provide it: Aether spoofs and attributes it.
        OwnershipFacts f = facts(false, 0, AppReleaseState::Invalid);
        const auto d = DecideOwnership(f);
        CHECK(d.action == OwnershipAction::Spoof);
        CHECK_EQ(d.ownerAccountId, 222u);
        CHECK(!d.steamOwned);
        CHECK(!d.steamFamilyShared);
    }
    {
        // Present in one package only: not Steam-provided (conservative).
        OwnershipFacts f = facts(true, 1, AppReleaseState::Released);
        const auto d = DecideOwnership(f);
        CHECK(d.action == OwnershipAction::Spoof);
        CHECK_EQ(d.ownerAccountId, 222u);
    }
    {
        // Not released yet: not Steam-provided.
        OwnershipFacts f = facts(true, 2, AppReleaseState::Uninstalled);
        const auto d = DecideOwnership(f);
        CHECK(d.action == OwnershipAction::Spoof);
    }
    {
        // No Aether depot: never touch Steam's answer.
        OwnershipFacts f = facts(true, 2, AppReleaseState::Released);
        f.hasDepot = false;
        const auto d = DecideOwnership(f);
        CHECK(d.action == OwnershipAction::PassThrough);
        CHECK(!d.steamOwned);
        CHECK(!d.steamFamilyShared);
    }
    {
        // Spoofing before the live account is known is allowed; the caller
        // leaves steamId32 alone when the owner is 0.
        OwnershipFacts f = facts(false, 0, AppReleaseState::Invalid);
        f.spoofOwnerAccountId = 0;
        const auto d = DecideOwnership(f);
        CHECK(d.action == OwnershipAction::Spoof);
        CHECK_EQ(d.ownerAccountId, 0u);
    }
    {
        // Sticky spoof: package 0 now makes Steam report owned, but Aether
        // already classified this app as not provided. Keep spoofing.
        OwnershipFacts f = facts(true, 2, AppReleaseState::Released);
        f.spoofed = true;
        const auto d = DecideOwnership(f);
        CHECK(d.action == OwnershipAction::Spoof);
        CHECK_EQ(d.ownerAccountId, 222u);
        CHECK(!d.steamOwned);
        CHECK(!d.steamFamilyShared);
        CHECK(!d.familyBanner);
    }
    {
        // Sticky family lender: the app is kept in package 0 for simultaneous
        // play, but the banner must name the real lender.
        OwnershipFacts f = facts(true, 2, AppReleaseState::Released);
        f.familyLenderAccountId = 333;
        f.spoofOwnerAccountId = 333; // caller resolves the lender for the banner
        const auto d = DecideOwnership(f);
        CHECK(d.action == OwnershipAction::Spoof);
        CHECK_EQ(d.ownerAccountId, 333u);
        CHECK(!d.steamOwned);
        CHECK(!d.steamFamilyShared);
        CHECK(d.familyBanner);
    }
    {
        // A sticky classification without Aether data is inert.
        OwnershipFacts f = facts(true, 2, AppReleaseState::Released);
        f.hasDepot = false;
        f.spoofed = true;
        f.familyLenderAccountId = 333;
        const auto d = DecideOwnership(f);
        CHECK(d.action == OwnershipAction::PassThrough);
    }

    SeedIncludesUnclassifiedLuaApp();
    PreSeededLuaAppStaysSpoofed();
    PreSeededOwnedAppIsMarkedOwned();
    PreSeededFamilyAppKeepsBanner();
    MembershipCountsTrackClassification();
    LibraryRootsExcludeDepots();
}
