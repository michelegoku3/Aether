#include "cases.h"

#include "core/OwnershipDecision.h"

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
}
