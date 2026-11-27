#include "cases.h"

#include <cstdint>
#include <vector>

#include "hooks/license/PackageSeedPolicy.h"

namespace {

// Package 0 is Steam's own object. The seed appends the ids a .lua declares and
// must leave everything Steam already wrote there exactly as it was: the ids it
// manages itself, their order, and even duplicates the seed does not name.
//
// The same transformation runs over both of package 0's id vectors - the app
// ids and the depot ids - so what it does to a plain list is the whole contract.

using ac::hooks::license::policy::ApplySeed;
using ac::hooks::license::policy::DecidePublishStep;
using ac::hooks::license::policy::NormalizeSeed;
using ac::hooks::license::policy::PublishStep;
using ac::steam::AppId;

void AppendsMissingIdsAtTheEnd() {
    const auto out = ApplySeed({10, 20}, {30, 40});
    CHECK(out.ids == (std::vector<AppId>{10, 20, 30, 40}));
    CHECK_EQ(out.appended, 2u);
    CHECK_EQ(out.presentBefore, 0u);
}

void KeepsSteamsOwnIdsInTheirOrder() {
    const auto out = ApplySeed({10, 20, 30}, {20});
    CHECK(out.ids == (std::vector<AppId>{10, 20, 30}));
    CHECK_EQ(out.appended, 0u);
    CHECK_EQ(out.presentBefore, 1u);
}

void DropsDuplicateCopiesOfSeededIds() {
    const auto out = ApplySeed({10, 10, 20}, {10});
    CHECK(out.ids == (std::vector<AppId>{10, 20}));
    CHECK_EQ(out.duplicatesRemoved, 1u);
}

void LeavesDuplicatesOfUnseededIdsAlone() {
    const auto out = ApplySeed({10, 10}, {20});
    CHECK(out.ids == (std::vector<AppId>{10, 10, 20}));
    CHECK_EQ(out.duplicatesRemoved, 0u);
}

void IgnoresZeroAndRepeatedSeedIds() {
    const auto out = ApplySeed({}, {0, 30, 30, 0, 40});
    CHECK(out.ids == (std::vector<AppId>{30, 40}));
    CHECK_EQ(out.appended, 2u);
}

void ApplyingTwiceChangesNothing() {
    const auto once = ApplySeed({10}, {20, 30});
    const auto twice = ApplySeed(once.ids, {20, 30});
    CHECK(twice.ids == once.ids);
    CHECK_EQ(twice.appended, 0u);
    CHECK_EQ(twice.duplicatesRemoved, 0u);
}

void NormalizeDropsZeroAndRepeats() {
    CHECK(NormalizeSeed({0, 5, 5, 0, 6}) == (std::vector<AppId>{5, 6}));
    CHECK(NormalizeSeed({}).empty());
}

void PublishTracksRefreshCompletion() {
    CHECK(DecidePublishStep(false, false, false) == PublishStep::Unseeded);
    CHECK(DecidePublishStep(true, true, false) == PublishStep::RefreshPending);
    CHECK(DecidePublishStep(true, true, true) == PublishStep::Published);
    CHECK(DecidePublishStep(true, false, false) == PublishStep::Published);
}

void PrunesFamilyRootFromBothVectorsKeepingDlcs() {
    // Package 0 carries both appIdVec and depotIdVec; Steam's
    // RebuildLicensesCache reads BOTH vectors into appToLicenses. Stripping a
    // borrowed root (2868840) must remove every copy from both vectors while
    // keeping companion DLC/depot ids (2868841, 2868850) and Steam's own ids.
    const std::vector<AppId> apps{7, 2868840, 2868850, 999999};
    const std::vector<AppId> depots{2868840, 2868841, 2868850, 777};
    const auto out = ac::hooks::license::policy::PrunePackageVectors(apps, depots, {2868840});
    CHECK(out.Changed());
    CHECK_EQ(out.apps.removed, 1u);
    CHECK_EQ(out.depots.removed, 1u);
    CHECK(out.apps.ids == (std::vector<AppId>{7, 2868850, 999999}));
    CHECK(out.depots.ids == (std::vector<AppId>{2868841, 2868850, 777}));

    // Re-pruning after removal is a no-op so callers do not spam license refreshes.
    const auto second = ac::hooks::license::policy::PrunePackageVectors(out.apps.ids, out.depots.ids, {2868840});
    CHECK(!second.Changed());
}

void ExcludeIdsDropsFamilyRootFromMixedDlcSeed() {
    // When a DLC lookup pulls in every id from a shared .lua file, excluding
    // the borrowed base app leaves all DLC/content depot ids intact.
    const auto filtered = ac::hooks::license::policy::ExcludeIds({0, 2868840, 2868841, 2868850, 2868840}, {2868840});
    CHECK(filtered == (std::vector<AppId>{2868841, 2868850}));
}

void PruneEligibilityCoversFamilyAndUnmaskWhileProtectingSpoofs() {
    using ac::hooks::license::policy::ShouldPruneRequestedApp;
    CHECK(ShouldPruneRequestedApp(/*isValveFamily=*/true, /*isSpoofed=*/false, /*isOwnBanner=*/false));
    CHECK(ShouldPruneRequestedApp(true, true, false));
    // Unclassified (unmask) and Steam-owned roots may be stripped from package 0.
    CHECK(ShouldPruneRequestedApp(false, false, false));
    // Active spoofs and opt-in banner stamps stay seeded in package 0.
    CHECK(!ShouldPruneRequestedApp(false, true, false));
    CHECK(!ShouldPruneRequestedApp(false, false, true));
}

} // namespace

void PackageSeedTest() {
    AppendsMissingIdsAtTheEnd();
    KeepsSteamsOwnIdsInTheirOrder();
    DropsDuplicateCopiesOfSeededIds();
    LeavesDuplicatesOfUnseededIdsAlone();
    IgnoresZeroAndRepeatedSeedIds();
    ApplyingTwiceChangesNothing();
    NormalizeDropsZeroAndRepeats();
    PublishTracksRefreshCompletion();
    PrunesFamilyRootFromBothVectorsKeepingDlcs();
    ExcludeIdsDropsFamilyRootFromMixedDlcSeed();
    PruneEligibilityCoversFamilyAndUnmaskWhileProtectingSpoofs();
}
