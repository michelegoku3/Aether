#include "cases.h"

#include "hooks/wire/DonorPool.h"
#include "hooks/wire/DonorProbe.h"

#include <ctime>

void DonorProbeTest() {

    namespace dp = ac::hooks::DonorPool;

    namespace pr = ac::hooks::wire::donorprobe;

    // Recording doubles: no wire, no disk.
    std::vector<std::size_t> sent;

    pr::SetSender([&sent](ac::steam::AppId, std::size_t donor) {
        sent.push_back(donor);

        return 0x9000000000000000ull + static_cast<std::uint64_t>(sent.size());
    });

    std::string published;

    pr::SetSink([&published](ac::steam::AppId, const std::string& schema) {
        published = schema;

        return true;
    });

    // The client already tried the default donor for this app.
    const ac::steam::AppId app = 999100u;

    dp::NoteAttemptResult(app, dp::kDefaultPoolIndex, /*okWithData=*/false);

    // Its failure starts a walk at the next untried slot.
    CHECK(pr::OnDonorWithoutData(app));

    CHECK(sent.size() == 1 && sent[0] == 15);

    // A second failure while a request is in flight must not fork the walk.
    CHECK(!pr::OnDonorWithoutData(app));

    CHECK(sent.size() == 1);

    // A donor without data advances to the next untried one.
    CHECK(!pr::OnProbeResult(app, 15, /*okWithData=*/false, "", /*eresult=*/2));

    CHECK(sent.size() == 2 && sent[1] == 0);

    // The first donor with a schema is pinned, published, and ends the walk.
    CHECK(pr::OnProbeResult(app, 1, /*okWithData=*/true, "SCHEMA", /*eresult=*/1));

    CHECK(published == "SCHEMA");

    CHECK(sent.size() == 2);

    CHECK(!pr::IsWalking(app));

    CHECK_EQ(dp::PickIndex(app), static_cast<std::size_t>(1));

    // A pool walked to the end without data is stamped as exhausted and left
    // alone: no probe storm on every session for an app nobody owns.
    const ac::steam::AppId dead = 999101u;

    for (std::size_t i = 0; i < dp::PoolCount(); ++i)
        dp::NoteAttemptResult(dead, i, /*okWithData=*/false);

    sent.clear();

    CHECK(!pr::OnDonorWithoutData(dead));

    CHECK(sent.empty());

    CHECK(dp::PoolExhaustedAt(dead) != 0u);

    // Once the retry window has elapsed the walk restarts from a fresh slot.
    dp::NotePoolExhausted(dead, static_cast<std::uint64_t>(std::time(nullptr)) -
                                    static_cast<std::uint64_t>(pr::kRetryWindowSeconds) - 1u);

    CHECK(pr::OnDonorWithoutData(dead));

    CHECK(sent.size() == 1 && sent[0] == dp::kDefaultPoolIndex);

    CHECK_EQ(dp::PoolExhaustedAt(dead), std::uint64_t{0});

    // A sender that cannot build the frame leaves no walk behind: the next
    // failure is free to try again.
    const ac::steam::AppId broken = 999102u;

    pr::SetSender([](ac::steam::AppId, std::size_t) { return std::uint64_t{0}; });

    CHECK(!pr::OnDonorWithoutData(broken));

    CHECK(!pr::IsWalking(broken));

    // Pacing: a send closer than the gap waits for the remainder, never longer.
    const auto t0 = std::chrono::steady_clock::now();

    CHECK(pr::ProbeDelay(t0, t0) == pr::kMinProbeGap);

    CHECK(pr::ProbeDelay(t0, t0 + std::chrono::milliseconds(100)) == pr::kMinProbeGap - std::chrono::milliseconds(100));

    CHECK(pr::ProbeDelay(t0, t0 + std::chrono::milliseconds(400)) == std::chrono::milliseconds(0));

    // Pacing is delegated, never slept on the recv thread: the probe asks the
    // scheduler for the remaining gap.
    std::vector<std::chrono::milliseconds> deferred;
    pr::SetSender([&sent](ac::steam::AppId, std::size_t donor) {
        sent.push_back(donor);
        return 0x9000000000000000ull + static_cast<std::uint64_t>(sent.size());
    });
    pr::SetScheduler([&deferred](std::chrono::milliseconds delay, std::function<void()> action) {
        deferred.push_back(delay);
        action();
    });
    const ac::steam::AppId paced = 999105u;
    CHECK(pr::OnDonorWithoutData(paced));
    CHECK(!pr::OnProbeResult(paced, dp::kDefaultPoolIndex, false, "", 2));
    CHECK(deferred.size() == 2);
    CHECK(deferred[0] > std::chrono::milliseconds(0));
    pr::SetScheduler({});

    // Rate limits are recognised and stop every walk for the session.
    CHECK(pr::IsRateLimited(84)); // k_EResultRateLimitExceeded
    CHECK(pr::IsRateLimited(25)); // k_EResultLimitExceeded
    CHECK(!pr::IsRateLimited(2));

    sent.clear();
    const ac::steam::AppId throttled = 999106u;
    CHECK(pr::OnDonorWithoutData(throttled));
    CHECK(sent.size() == 1);
    CHECK(!pr::OnProbeResult(throttled, dp::kDefaultPoolIndex, false, "", 84));
    CHECK(sent.size() == 1);
    CHECK(pr::IsThrottled());
    CHECK(!pr::OnDonorWithoutData(999107u)); // other apps too
}
