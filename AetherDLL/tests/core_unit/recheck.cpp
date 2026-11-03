#include "cases.h"

#include "hooks/steamui/AppRecheckPolicy.h"

namespace {

// A version apply rewrites the Lua and nothing else, so the client keeps the
// manifest ids it read at startup until something asks it to look again. These
// cases pin which apps that request names, and the queue that carries it.

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

} // namespace

void RecheckTest() {
    NoRecheckWhenPinsUnchanged();
    RecheckWhenAPinMoves();
    RecheckWhenADepotIsAdded();
    RecheckIsDeduplicated();
    QueueIsBounded();
}
