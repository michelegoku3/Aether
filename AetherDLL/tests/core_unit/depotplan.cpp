#include "cases.h"

#include <cstdint>
#include <vector>

#include "hooks/steamclient/DepotPlan.h"
#include "scripting/LuaData.h"

namespace {

// Steam plans depot rows only for the depots its AppInfo lists. A .lua app
// installed without AppInfo therefore plans just the shared/dependency rows,
// which is the "251 MB instead of 118 GB" download. The pins from the .lua
// files are the only complete depot list those apps have.

// Only the pins attributed to that app's own .lua file: a file that lists
// another app's depots must not inflate the plan.
void PinnedDepotsForAppIgnoresOtherFiles() {
    const char* fc6 = "C:/fixtures/2369390.lua";
    ac::luadata::BeginFile(fc6);
    ac::luadata::RecordLibraryApp(2369390);
    ac::luadata::RecordDepot(2369391, "");
    ac::luadata::SetManifestOverride(2369391, {5783041166523199015ull, 118932477611ull});
    ac::luadata::EndFile();

    const char* re2 = "C:/fixtures/883710.lua";
    ac::luadata::BeginFile(re2);
    ac::luadata::RecordLibraryApp(883710);
    ac::luadata::RecordDepot(883711, "");
    ac::luadata::SetManifestOverride(883711, {1276883776777242280ull, 27213723651ull});
    ac::luadata::EndFile();

    const auto pins = ac::luadata::PinnedDepotsForApp(2369390);
    CHECK(pins.size() == 1);
    CHECK(pins[0].depotId == 2369391);
    CHECK_EQ(pins[0].gid, 5783041166523199015ull);
    CHECK_EQ(pins[0].size, 118932477611ull);

    ac::luadata::UnloadFile(fc6);
    ac::luadata::UnloadFile(re2);
}

void ComputeMissingPinsSkipsKnownDepots() {
    const std::vector<ac::luadata::DepotPin> pins{
        {2369391, 5783041166523199015ull, 118932477611ull},
        {2369392, 8390405336387105870ull, 1ull},
        {1716751, 8458906404271031972ull, 264202144ull},
    };
    const auto missing = ac::hooks::ComputeMissingPins(pins, {1716751u});
    CHECK(missing.size() == 2);
    CHECK(missing[0].depotId == 2369391);
    CHECK(missing[1].depotId == 2369392);
}

void ComputeMissingPinsDropsZeroGid() {
    const std::vector<ac::luadata::DepotPin> pins{{883719, 0ull, 0ull}};
    CHECK(ac::hooks::ComputeMissingPins(pins, {}).empty());
}

void ComputeMissingPinsIsBounded() {
    std::vector<ac::luadata::DepotPin> pins;
    for (ac::steam::AppId id = 1; id <= 600; ++id)
        pins.push_back({id, 1000ull + id, 1ull});
    const auto missing = ac::hooks::ComputeMissingPins(pins, {});
    CHECK(missing.size() == ac::hooks::kMaxAppendedDepots);
    CHECK(missing.size() == 512);
}

} // namespace

void DepotPlanTest() {
    PinnedDepotsForAppIgnoresOtherFiles();
    ComputeMissingPinsSkipsKnownDepots();
    ComputeMissingPinsDropsZeroGid();
    ComputeMissingPinsIsBounded();
}
