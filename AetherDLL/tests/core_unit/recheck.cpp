#include "cases.h"

#include <cstddef>
#include <string>
#include <vector>

#include "hooks/steamui/AppOverviewPolicy.h"
#include "hooks/steamui/AppRecheckPolicy.h"
#include "scripting/LuaData.h"

namespace {

// A version apply rewrites the Lua and nothing else, so the client keeps the
// manifest ids it read at startup until something asks it to look again. These
// cases pin which apps that request names, the queue that carries it, and the
// snapshot the reload diff is taken from.

using ac::steam::AppId;
using namespace ac::hooks::steamui::policy;

void NoRecheckWhenPinsUnchanged() {
    PinChange c{100, {{200, 1}, {201, 2}}, {{200, 1}, {201, 2}}};
    CHECK(SelectRecheck({c}).empty());
}

void RecheckWhenAPinMoves() {
    PinChange c{100, {{200, 1}}, {{200, 2}}};
    CHECK(SelectRecheck({c}) == std::vector<AppId>{100});
}

void RecheckWhenADepotIsAdded() {
    PinChange c{100, {{200, 1}}, {{200, 1}, {201, 5}}};
    CHECK(SelectRecheck({c}) == std::vector<AppId>{100});
}

// The shape a removed .lua leaves behind: the file contributes no pins at all,
// and every app it used to pin still has to be named or its card keeps the build
// the client read before the removal.
void RecheckWhenEveryPinGoesAway() {
    PinChange c{100, {{200, 1}, {201, 2}}, {}};
    CHECK(SelectRecheck({c}) == std::vector<AppId>{100});
}

// The snapshot the diff is taken from: pins belong to the file that states them,
// and each library root that file advertises carries them. Sorted, so a reload
// that only reordered `setmanifestid` lines compares equal.
void PinsFollowTheFileAndItsRoots() {
    const std::string path = "C:/fixtures/recheck_pins.lua";
    ac::luadata::BeginFile(path);
    ac::luadata::RecordLibraryApp(100);
    ac::luadata::RecordLibraryApp(101);
    // Recorded out of order on purpose: the snapshot is sorted by depot, so
    // 201 first proves the accessor does the ordering rather than the script.
    ac::luadata::RecordDepot(201, "");
    ac::luadata::SetManifestOverride(201, {12, 0});
    ac::luadata::RecordDepot(200, "");
    ac::luadata::SetManifestOverride(200, {11, 0});
    ac::luadata::EndFile();

    const auto pins = ac::luadata::PinsByLibraryAppForFile(path);
    CHECK(pins.size() == 2);
    const std::vector<ac::luadata::PinnedDepot> expected{{200, 11}, {201, 12}};
    for (const AppId app : {AppId{100}, AppId{101}}) {
        CHECK(pins.count(app) == 1);
        const auto& rows = pins.at(app);
        CHECK(rows.size() == expected.size());
        for (std::size_t i = 0; i < expected.size(); ++i) {
            CHECK_EQ(rows[i].depot, expected[i].depot);
            CHECK_EQ(rows[i].gid, expected[i].gid);
        }
    }

    // A file the store no longer holds answers empty; that absence is how the
    // reload diff observes the pins going away.
    ac::luadata::UnloadFile(path);
    CHECK(ac::luadata::PinsByLibraryAppForFile(path).empty());
    ac::luadata::ClearPendingChanges();
}

void RecheckIsDeduplicated() {
    // Two changes for one app are still one recheck: the client rebuilds the
    // whole app, so asking twice would only repeat the work.
    PinChange a{100, {{200, 1}}, {{200, 2}}};
    PinChange b{100, {{200, 2}}, {{200, 3}}};
    CHECK(SelectRecheck({a, b}) == std::vector<AppId>{100});
}

void QueueIsBounded() {
    RecheckQueue q;
    for (std::size_t i = 0; i < RecheckQueue::kCapacity + 10; ++i)
        q.Push(static_cast<AppId>(1000 + i));
    CHECK(q.Size() == RecheckQueue::kCapacity);
    AppId out = 0;
    CHECK(q.Pop(out)); // oldest first
    CHECK(out == 1000);

    // Still in flight, so it is not queued a second time.
    CHECK(!q.Push(1001));
    CHECK(q.Size() == RecheckQueue::kCapacity - 1);

    // 1000 was popped, so there is room for it again.
    q.Push(1000);
    CHECK(q.Size() == RecheckQueue::kCapacity);

    // Pop on an empty queue reports "nothing to do" instead of touching `out`.
    RecheckQueue fresh;
    AppId none = 0;
    CHECK(!fresh.Pop(none));
    CHECK(none == 0);
}

void RemovedRootsFollowTheConfiguredBatch() {
    // Nothing was advertised: a removal has nothing to retire.
    CHECK(RetiredRoots({}, {}).empty());

    // A root the batch does not configure any more is retired; one another file
    // still advertises is not.
    CHECK(RetiredRoots({100, 101}, {101}) == std::vector<AppId>{100});

    // Duplicates collapse and the answer is ordered, so the log and the queue
    // do not depend on the order the snapshot happened to hold.
    CHECK(RetiredRoots({101, 100, 100}, {}) == (std::vector<AppId>{100, 101}));

    // Atomic replace (temp file + rename): the new file was parsed before this
    // runs, so the root is configured again and nothing is retired.
    CHECK(RetiredRoots({100}, {100}).empty());
}

} // namespace

void RecheckTest() {
    NoRecheckWhenPinsUnchanged();
    RecheckWhenAPinMoves();
    RecheckWhenADepotIsAdded();
    RecheckWhenEveryPinGoesAway();
    PinsFollowTheFileAndItsRoots();
    RecheckIsDeduplicated();
    QueueIsBounded();
    RemovedRootsFollowTheConfiguredBatch();
}
