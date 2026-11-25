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
}
