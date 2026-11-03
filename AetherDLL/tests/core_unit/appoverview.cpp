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

    CHECK(!DecideOwnershipWrite(input(false, false, true, true, true, 0)).write); // not managed
    CHECK(!DecideOwnershipWrite(input(true, true, true, true, true, 0)).write);   // family shared
    CHECK(!DecideOwnershipWrite(input(true, false, false, true, true, 0)).write); // card rejected
    CHECK(!DecideOwnershipWrite(input(true, false, true, false, true, 0)).write); // probe unproven
    CHECK(!DecideOwnershipWrite(input(true, false, true, true, false, 0)).write); // layout contradicted
    CHECK(!DecideOwnershipWrite(input(true, false, true, true, true, 1)).write);  // already owned

    const auto decision = DecideOwnershipWrite(input(true, false, true, true, true, 0));
    CHECK(decision.write);
    CHECK(std::string(decision.reason) == "owned");

    // The write is idempotent and preserves every other bit.
    CHECK(ApplyOwned(0) == 1u);
    CHECK(ApplyOwned(1) == 1u);
    CHECK(ApplyOwned(0x20) == 0x21u);
}

} // namespace

void AppOverviewTest() {
    RejectsObjectsThatAreNotTheOverview();
    LatchNeedsTwoDistinctApps();
    OwnershipWriteTable();
}
