#include "cases.h"

#include <cstdint>

#include "hooks/steamclient/DepotPlanPolicy.h"

namespace {

// Steam rebuilds an app's depot plan from its AppInfo. While the client swaps
// that AppInfo in after a PICS refresh it answers with zero depots for a moment,
// and its caller stores whatever comes back. An empty plan is an app that
// "installs" instantly because there is nothing to download, and that is then
// stamped Fully Installed with no size on disk.
//
// These cases pin when Aether refuses that answer. The shared/dependency vector
// is not an input on purpose: its rows belong to other apps, so a row there
// cannot make an empty own list look planned.

using ac::hooks::steamclient::policy::DecidePlan;
using ac::hooks::steamclient::policy::GrowCoversRows;
using ac::hooks::steamclient::policy::GrowSlotsFor;
using ac::hooks::steamclient::policy::ParseSize;
using ac::hooks::steamclient::policy::PlanAnswer;
using ac::hooks::steamclient::policy::PlanVerdict;

void RefusesAnEmptyPlanForAManagedApp() {
    CHECK(DecidePlan(PlanAnswer{.ownRows = 0, .managed = true, .steamOk = true}) == PlanVerdict::Refuse);
}

void KeepsAPlanWithRows() {
    CHECK(DecidePlan(PlanAnswer{.ownRows = 1, .managed = true, .steamOk = true}) == PlanVerdict::Keep);
}

void KeepsAnEmptyPlanForAnUnmanagedApp() {
    // No `.lua` names this app, so an empty plan is Steam's own answer for it.
    // Refusing would leave a config the user cannot clear from Aether.
    CHECK(DecidePlan(PlanAnswer{.ownRows = 0, .managed = false, .steamOk = true}) == PlanVerdict::Keep);
}

void KeepsSteamsOwnFailure() {
    // Steam already failed the call; its caller keeps the previous config on
    // that path, so there is nothing to refuse and nothing to log.
    CHECK(DecidePlan(PlanAnswer{.ownRows = 0, .managed = true, .steamOk = false}) == PlanVerdict::Keep);
}

// --- Refill ----------------------------------------------------------------
//
// The plan is not only a race. On current builds Steam also answers with no own
// rows for a managed app whose content it does not license: the rows it does
// keep belong to other apps (shared redistributables), and the app reports 0 B.
// The `.lua` pins say exactly which depots the app is made of, so the rows can
// be put back. These cases pin which ones go back and in what order.

using ac::hooks::steamclient::policy::PinnedDepot;
using ac::hooks::steamclient::policy::SelectRefillRows;
using ac::steam::AppId;

void RefillsAnEmptyPlanFromThePins() {
    const auto rows = SelectRefillRows(100, {}, {{101, 11, 1100}, {102, 12, 1200}});
    CHECK(rows.size() == 2u);
    CHECK_EQ(rows[0].depotId, 101u);
    CHECK_EQ(rows[0].gid, 11u);
    CHECK_EQ(rows[0].size, 1100u);
    CHECK_EQ(rows[1].depotId, 102u);
}

void NeverAddsARowSteamAlreadyPlanned() {
    // Steam's own row carries the gid of the branch it selected; a second copy
    // for the same depot would download it twice.
    const auto rows = SelectRefillRows(100, {101}, {{101, 11, 1100}, {102, 12, 1200}});
    CHECK(rows.size() == 1u);
    CHECK_EQ(rows[0].depotId, 102u);
}

void NeverAddsTheAppItself() {
    // The root app is not a depot; a row for it would ask Steam to download the
    // app as content.
    const auto rows = SelectRefillRows(100, {}, {{100, 11, 1100}});
    CHECK(rows.empty());
}

void SkipsDepotsWithoutAPin() {
    // No gid means no manifest to install. A row with gid 0 would be a depot
    // Steam cannot resolve.
    const auto rows = SelectRefillRows(100, {}, {{101, 0, 0}, {102, 12, 1200}});
    CHECK(rows.size() == 1u);
    CHECK_EQ(rows[0].depotId, 102u);
}

void CollapsesRepeatedPinsKeepingTheFirst() {
    const auto rows = SelectRefillRows(100, {}, {{102, 12, 1200}, {102, 99, 9900}});
    CHECK(rows.size() == 1u);
    CHECK_EQ(rows[0].gid, 12u);
}

void OrdersRowsByDepotIdWhateverTheInputOrder() {
    const auto rows = SelectRefillRows(100, {}, {{105, 15, 1}, {101, 11, 1}, {103, 13, 1}});
    CHECK(rows.size() == 3u);
    CHECK_EQ(rows[0].depotId, 101u);
    CHECK_EQ(rows[1].depotId, 103u);
    CHECK_EQ(rows[2].depotId, 105u);
}

void RefillingTwiceAsksForNothingNew() {
    const std::vector<AppId> plan{101, 102};
    CHECK(SelectRefillRows(100, plan, {{101, 11, 1}, {102, 12, 1}}).empty());
}

void GrowRequestCoversTheRowsItMustHold() {
    // The pattern table's grow helper is the uint32 instantiation: it charges
    // four bytes per element. A plan row is 32 bytes, so one row needs eight
    // requested elements.
    CHECK_EQ(GrowSlotsFor(0), 0u);
    CHECK_EQ(GrowSlotsFor(1), 8u);
    CHECK_EQ(GrowSlotsFor(9), 72u);
    CHECK_EQ(GrowSlotsFor(28), 224u);
    CHECK(GrowCoversRows(GrowSlotsFor(28), 28));
    // One element short leaves the last row without bytes to live in.
    CHECK(!GrowCoversRows(GrowSlotsFor(28) - 1, 28));
}

void CapacityIsMeasuredInBytesNotRows() {
    // A plan of one existing row plus eight pinned ones needs nine rows of room,
    // and the helper reports what it reserved in uint32 elements.
    const std::uint32_t rows = 1 + 8;
    CHECK(!GrowCoversRows(rows, rows)); // what a row-wise request would reserve
    CHECK(GrowCoversRows(GrowSlotsFor(rows), rows));
}

void ReadsTheSizeAppInfoAnswersWith() {
    // AppInfo answers a manifest's size as a decimal string, sometimes NUL
    // padded to the caller's buffer.
    CHECK_EQ(ParseSize("27213723651", 11), 27213723651ull);
    CHECK_EQ(ParseSize("42", 2), 42ull);
    CHECK_EQ(ParseSize("0", 1), 0ull);
    // Junk stops the read rather than becoming a size.
    CHECK_EQ(ParseSize("", 0), 0ull);
    CHECK_EQ(ParseSize("abc", 3), 0ull);
    CHECK_EQ(ParseSize("12x7", 4), 12ull);
}

} // namespace

void DepotPlanTest() {
    RefusesAnEmptyPlanForAManagedApp();
    KeepsAPlanWithRows();
    KeepsAnEmptyPlanForAnUnmanagedApp();
    KeepsSteamsOwnFailure();
    RefillsAnEmptyPlanFromThePins();
    NeverAddsARowSteamAlreadyPlanned();
    NeverAddsTheAppItself();
    SkipsDepotsWithoutAPin();
    CollapsesRepeatedPinsKeepingTheFirst();
    OrdersRowsByDepotIdWhateverTheInputOrder();
    RefillingTwiceAsksForNothingNew();
    GrowRequestCoversTheRowsItMustHold();
    CapacityIsMeasuredInBytesNotRows();
    ReadsTheSizeAppInfoAnswersWith();
}
