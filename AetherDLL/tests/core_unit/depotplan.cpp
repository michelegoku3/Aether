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
using ac::hooks::steamclient::policy::DecideRefill;
using ac::hooks::steamclient::policy::GrowCoversRows;
using ac::hooks::steamclient::policy::GrowSlotsFor;
using ac::hooks::steamclient::policy::ParseSize;
using ac::hooks::steamclient::policy::PlanAnswer;
using ac::hooks::steamclient::policy::PlanVerdict;
using ac::hooks::steamclient::policy::PromotePinnedTarget;
using ac::hooks::steamclient::policy::RecordedDepot;
using ac::hooks::steamclient::policy::RefillSource;
using ac::hooks::steamclient::policy::RowSize;
using ac::hooks::steamclient::policy::SelectRecordedRows;

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

void RestoresTheDepotsAnInstalledAppRecords() {
    // An installed app's appmanifest records what is on disk, languages
    // included, so the plan is restored from it rather than from the `.lua`:
    // filling it from the `.lua` would mark each language pack as missing
    // content and the client would download it.
    CHECK(DecideRefill(true, 13, false) == RefillSource::Recorded);
    // Nothing installed: the plan is the only thing that can size the dialog.
    CHECK(DecideRefill(false, 13, false) == RefillSource::Pins);
    // No depot to put back.
    CHECK(DecideRefill(false, 0, false) == RefillSource::None);
}

void LeavesARealFamilyShareToSteam() {
    // The licence is Steam's, and its answer already plans the depots the
    // account may use: a row added here would claim content it does not.
    const std::vector<RefillSource> sources{DecideRefill(true, 13, true), DecideRefill(false, 13, true)};
    CHECK(sources[0] == RefillSource::None);
    CHECK(sources[1] == RefillSource::None);
}

// --- Recorded depots --------------------------------------------------------
//
// An installed app's plan is what keeps its content on disk: Steam removes a
// depot the plan leaves out and deletes the files only that depot carried. The
// client answers with no own rows for a managed app it does not license, even
// straight after its own commit wrote the ACF, so the depots the ACF records
// must go back into the plan. The `.lua` still selects the build: a pin wins
// over the recorded gid.

void SelectsOnlyTheDepotsTheAcfRecords() {
    // The `.lua` lists every depot the store has, language packs included; only
    // what the ACF records is content this install already carries.
    const std::vector<RecordedDepot> recorded{{101, 11, 1100}, {102, 12, 1200}};
    const std::vector<PinnedDepot> pinned{{101, 0, 0}, {102, 0, 0}, {103, 13, 1300}};
    const auto rows = SelectRecordedRows(100, {}, pinned, recorded);
    CHECK(rows.size() == 2u);
    CHECK_EQ(rows[0].depotId, 101u);
    CHECK_EQ(rows[1].depotId, 102u);
}

void UsesThePinForARecordedDepot() {
    // A version apply moves the pin and nothing else: the row must name the
    // build the user picked, not the gid Steam installed last time.
    const std::vector<RecordedDepot> recorded{{101, 11, 1100}};
    const std::vector<PinnedDepot> pinned{{101, 77, 7700}};
    const auto rows = SelectRecordedRows(100, {}, pinned, recorded);
    CHECK(rows.size() == 1u);
    CHECK_EQ(rows[0].gid, 77u);
    CHECK_EQ(rows[0].size, 7700u);
}

void FallsBackToTheRecordedGidWithoutAPin() {
    // AppInfo is mid-swap and the `.lua` leaves this depot unpinned: the ACF
    // names the manifest the content came from, which is enough for a row.
    const std::vector<RecordedDepot> recorded{{101, 11, 1100}};
    const auto rows = SelectRecordedRows(100, {}, {}, recorded);
    CHECK(rows.size() == 1u);
    CHECK_EQ(rows[0].gid, 11u);
    CHECK_EQ(rows[0].size, 1100u);
}

void KeepsTheRecordedSizeWhenThePinHasNone() {
    const std::vector<RecordedDepot> recorded{{101, 11, 1100}};
    const std::vector<PinnedDepot> pinned{{101, 77, 0}};
    const auto rows = SelectRecordedRows(100, {}, pinned, recorded);
    CHECK(rows.size() == 1u);
    CHECK_EQ(rows[0].gid, 77u);
    CHECK_EQ(rows[0].size, 1100u);
}

void NeverAddsARecordedDepotSteamAlreadyPlanned() {
    const std::vector<RecordedDepot> recorded{{101, 11, 1100}, {102, 12, 1200}};
    const auto rows = SelectRecordedRows(100, {101}, {}, recorded);
    CHECK(rows.size() == 1u);
    CHECK_EQ(rows[0].depotId, 102u);
}

void SkipsRecordedDepotsWithoutAGid() {
    // No gid means no manifest to install; a row would be a depot Steam cannot
    // resolve.
    const std::vector<RecordedDepot> recorded{{101, 0, 0}};
    CHECK(SelectRecordedRows(100, {}, {}, recorded).empty());
}

void OrdersRecordedRowsByDepotId() {
    const std::vector<RecordedDepot> recorded{{105, 15, 1}, {101, 11, 1}, {103, 13, 1}};
    const auto rows = SelectRecordedRows(100, {}, {}, recorded);
    CHECK(rows.size() == 3u);
    CHECK_EQ(rows[0].depotId, 101u);
    CHECK_EQ(rows[1].depotId, 103u);
    CHECK_EQ(rows[2].depotId, 105u);
}

void NeverAddsTheRecordedAppItself() {
    // The root app is not content, even if a corrupt record names it.
    const std::vector<RecordedDepot> recorded{{100, 11, 1100}};
    CHECK(SelectRecordedRows(100, {}, {}, recorded).empty());
}

void CollapsesRepeatedRecordedDepotsKeepingTheFirst() {
    const std::vector<RecordedDepot> recorded{{101, 11, 1100}, {101, 99, 9900}};
    const auto rows = SelectRecordedRows(100, {}, {}, recorded);
    CHECK(rows.size() == 1u);
    CHECK_EQ(rows[0].gid, 11u);
}

void PromotesAPinnedRowSteamCarriedAsInstalledOnly() {
    // Steam hands a depot that is already on disk over as an installed-only
    // row, and its update job compares target rows only: a pin that names
    // another gid has to become the target, or it is mounted and recorded as
    // installed without a byte downloaded.
    CHECK(PromotePinnedTarget(22, 11, true));
    CHECK(!PromotePinnedTarget(11, 11, true));
    CHECK(!PromotePinnedTarget(22, 11, false));
    CHECK(!PromotePinnedTarget(0, 11, true));
    CHECK(!PromotePinnedTarget(22, 0, true));
}

void RowSizePrefersThePinThenAppInfo() {
    // The row's own size is the pin's or the ACF's; AppInfo answers only when
    // neither has one. Zero stays zero rather than becoming a guess.
    const std::vector<std::uint64_t> sizes{RowSize(PinnedDepot{101, 11, 1100}, 0),
                                           RowSize(PinnedDepot{101, 11, 0}, 1200), RowSize(PinnedDepot{101, 11, 0}, 0)};
    CHECK_EQ(sizes[0], 1100u);
    CHECK_EQ(sizes[1], 1200u);
    CHECK_EQ(sizes[2], 0u);
}

// --- Platform --------------------------------------------------------------
//
// A `.lua` lists every depot the store has, including the ones only another
// platform runs, and two platforms can declare the same file path. AppInfo
// answers `depots/<id>/config/oslist` for each depot; the plan keeps only the
// rows this client can run. These cases pin the list parsing, including the
// absent key, which must never remove content.

using ac::hooks::steamclient::policy::OsListAllowsHost;

void KeepsADepotThatRunsHere() { CHECK(OsListAllowsHost("windows", "windows")); }

void DropsADepotOnlyAnotherPlatformRuns() {
    CHECK(!OsListAllowsHost("linux", "windows"));
    CHECK(!OsListAllowsHost("macos", "windows"));
}

void ReadsTheWholeListWhateverItsOrder() {
    CHECK(OsListAllowsHost("windows,linux", "windows"));
    CHECK(OsListAllowsHost("linux,windows", "windows"));
    CHECK(OsListAllowsHost(" linux , windows ", "windows"));
    CHECK(OsListAllowsHost("windows,", "windows"));
}

void IgnoresCase() { CHECK(OsListAllowsHost("Windows", "windows")); }

void TreatsAnAbsentListAsUnrestricted() { CHECK(OsListAllowsHost("", "windows")); }

void MatchesWholeTokensOnly() {
    CHECK(!OsListAllowsHost("windowsx", "windows"));
    CHECK(!OsListAllowsHost("notwindows", "windows"));
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
    RestoresTheDepotsAnInstalledAppRecords();
    SelectsOnlyTheDepotsTheAcfRecords();
    UsesThePinForARecordedDepot();
    FallsBackToTheRecordedGidWithoutAPin();
    KeepsTheRecordedSizeWhenThePinHasNone();
    NeverAddsARecordedDepotSteamAlreadyPlanned();
    SkipsRecordedDepotsWithoutAGid();
    OrdersRecordedRowsByDepotId();
    NeverAddsTheRecordedAppItself();
    CollapsesRepeatedRecordedDepotsKeepingTheFirst();
    PromotesAPinnedRowSteamCarriedAsInstalledOnly();
    RowSizePrefersThePinThenAppInfo();
    KeepsADepotThatRunsHere();
    DropsADepotOnlyAnotherPlatformRuns();
    ReadsTheWholeListWhateverItsOrder();
    IgnoresCase();
    TreatsAnAbsentListAsUnrestricted();
    MatchesWholeTokensOnly();
    LeavesARealFamilyShareToSteam();
}
