#include "cases.h"

#include "hooks/steamui/AppOverviewPolicy.h"

namespace {

// The SteamUI app-overview object is written by Aether only after this policy
// agrees that the memory under the compiled offsets is really that object.
// These cases pin the tripwires: a false accept would write into whatever the
// layout moved to, which is exactly the failure this layer exists to avoid.

using ac::steam::AppOverviewPrefix;
using ac::steam::AppOwnershipFlags;
using namespace ac::hooks::steamui::policy;

AppOverviewPrefix ConsistentApp(ac::steam::AppId appId) {
    AppOverviewPrefix app{};
    app.gameId = appId;
    app.appId = appId;
    app.ownershipFlags = AppOwnershipFlags::None;
    app.appStateFlags = 0x4; // FullyInstalled
    return app;
}

void RejectsObjectsThatAreNotTheOverview() {
    CHECK(InspectOverview(nullptr) == OverviewVerdict::NullObject);

    auto app = ConsistentApp(0x1234);
    CHECK(InspectOverview(&app) == OverviewVerdict::Ok);

    app.appId = 0;
    CHECK(InspectOverview(&app) == OverviewVerdict::AppIdZero);

    // gameId and appId disagree: this is not the object we compiled against.
    app = ConsistentApp(0x1234);
    app.appId = 0x1235;
    CHECK(InspectOverview(&app) == OverviewVerdict::GameIdMismatch);

    // A flag outside the published masks is a layout move, not a new bit.
    app = ConsistentApp(0x1234);
    app.ownershipFlags = static_cast<AppOwnershipFlags>(0x80000000u);
    CHECK(InspectOverview(&app) == OverviewVerdict::OwnershipOutOfMask);

    app = ConsistentApp(0x1234);
    app.appStateFlags = 0x04000000u;
    CHECK(InspectOverview(&app) == OverviewVerdict::StateOutOfMask);
}

void LatchNeedsTwoDistinctApps() {
    auto app = ConsistentApp(0x1234);
    ProbeState probe{};

    CHECK(!RecordProbeSample(probe, app));
    CHECK(!RecordProbeSample(probe, app));
    CHECK(!probe.latched);

    // A second, different app id is what distinguishes "an object" from "the
    // object we think it is": one layout could match by coincidence.
    auto second = ConsistentApp(0x2222);
    CHECK(RecordProbeSample(probe, second));
    CHECK(probe.latched);
    // Latching is one-way for the session.
    CHECK(!RecordProbeSample(probe, second));
    CHECK(probe.latched);
}

void OwnershipWriteTable() {
    auto input = [](bool managed, bool familyShared, bool sampleOk, bool latched, bool trusted, std::uint32_t flags) {
        WriteInputs in{};
        in.managed = managed;
        in.familyShared = familyShared;
        in.sampleOk = sampleOk;
        in.probeLatched = latched;
        in.layoutTrusted = trusted;
        in.currentFlags = flags;
        return in;
    };

    CHECK(DecideOwnershipWrite(input(false, false, true, true, true, 0)).action == OwnershipAction::None);
    CHECK(DecideOwnershipWrite(input(true, true, true, true, true, 0)).action == OwnershipAction::None);
    CHECK(DecideOwnershipWrite(input(true, false, false, true, true, 0)).action == OwnershipAction::None);
    CHECK(DecideOwnershipWrite(input(true, false, true, false, true, 0)).action == OwnershipAction::None);
    CHECK(DecideOwnershipWrite(input(true, false, true, true, false, 0)).action == OwnershipAction::None);
    CHECK(DecideOwnershipWrite(input(true, false, true, true, true, 1)).action == OwnershipAction::None);

    const auto decision = DecideOwnershipWrite(input(true, false, true, true, true, 0));
    CHECK(decision.action == OwnershipAction::Stamp);
    CHECK(std::string(decision.reason) == "owned");

    // A stamp that cannot be remembered would be unclearable: refuse it.
    auto full = input(true, false, true, true, true, 0);
    full.trackingFull = true;
    const auto refused = DecideOwnershipWrite(full);
    CHECK(refused.action == OwnershipAction::None);
    CHECK(std::string(refused.reason) == "tracking-full");

    // The write is idempotent and preserves every other bit.
    CHECK(ApplyOwned(0) == 1u);
    CHECK(ApplyOwned(1) == 1u);
    CHECK(ApplyOwned(0x20) == 0x21u);
}

// The reverse write. The owned bit lives in the client's object, so it outlives
// the licence that justified it; a removal must take it back, but only from an
// id Aether stamped, and only for a card the probe confirmed.
void OwnershipClearTable() {
    auto retired = [](bool stamped, bool stillManaged, bool familyShared, std::uint32_t flags) {
        WriteInputs in{};
        in.retired = true;
        in.stamped = stamped;
        in.managed = stillManaged;
        in.familyShared = familyShared;
        in.sampleOk = true;
        in.probeLatched = true;
        in.layoutTrusted = true;
        in.currentFlags = flags;
        return in;
    };

    // The card Aether stamped, no longer configured, still showing the bit.
    const auto decision = DecideOwnershipWrite(retired(true, false, false, 1));
    CHECK(decision.action == OwnershipAction::Clear);
    CHECK(std::string(decision.reason) == "retired");

    // Never clear a bit Aether did not write: a genuine licence also reports
    // "not managed", and its bit belongs to Valve.
    CHECK(DecideOwnershipWrite(retired(false, false, false, 1)).action == OwnershipAction::None);
    // Nothing to clear, so nothing to report either.
    CHECK(DecideOwnershipWrite(retired(true, false, false, 0)).action == OwnershipAction::None);
    CHECK(std::string(DecideOwnershipWrite(retired(true, false, false, 0)).reason) == "already-clear");
    // Retired but configured again (the file came back): the stamp path owns it.
    CHECK(DecideOwnershipWrite(retired(true, true, false, 1)).action == OwnershipAction::None);
    CHECK(DecideOwnershipWrite(retired(true, true, false, 0)).action == OwnershipAction::Stamp);
    // A family answer keeps the client's bit.
    CHECK(DecideOwnershipWrite(retired(true, false, true, 1)).action == OwnershipAction::None);

    // A clear needs the same layout evidence as a stamp.
    auto unproven = retired(true, false, false, 1);
    unproven.probeLatched = false;
    CHECK(DecideOwnershipWrite(unproven).action == OwnershipAction::None);
    auto rejected = retired(true, false, false, 1);
    rejected.sampleOk = false;
    CHECK(DecideOwnershipWrite(rejected).action == OwnershipAction::None);
    auto contradicted = retired(true, false, false, 1);
    contradicted.layoutTrusted = false;
    CHECK(DecideOwnershipWrite(contradicted).action == OwnershipAction::None);

    // Clearing preserves every other bit.
    CHECK(ApplyCleared(1) == 0u);
    CHECK(ApplyCleared(0x21) == 0x20u);
    CHECK(ApplyCleared(0) == 0u);
}

} // namespace

void AppOverviewTest() {
    RejectsObjectsThatAreNotTheOverview();
    LatchNeedsTwoDistinctApps();
    OwnershipWriteTable();
    OwnershipClearTable();
}
