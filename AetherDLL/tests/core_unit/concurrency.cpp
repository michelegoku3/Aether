#include "cases.h"

#include "core/Workers.h"

#include "utils/CoalescingWorker.h"

#include "utils/LogBurstBudget.h"

#include "utils/TtlCache.h"

void Worker() {

    ac::utils::CoalescingWorker worker;

    std::atomic<std::uint64_t> requests{0}, callbacks{0};

    std::atomic<int> active{0}, maxActive{0};

    worker.Start([&](std::uint64_t n) {
        const auto running = ++active;

        if (running > maxActive)

            maxActive = running;

        requests += n;

        ++callbacks;

        --active;
    });

    std::vector<std::thread> threads;

    for (int i = 0; i < 8; ++i)

        threads.emplace_back([&] {
            for (int j = 0; j < 1000; ++j)

                worker.Request();
        });

    for (auto& t : threads)

        t.join();

    worker.Stop(); // drains pending requests, including when no timer fired yet

    CHECK(requests == 8000);

    CHECK(callbacks > 0 && callbacks < 8000);

    CHECK(maxActive == 1);

    worker.Request();

    worker.Stop();

    CHECK(requests == 8000);

    // Requests arriving DURING serialization must get another write.

    ac::utils::CoalescingWorker reentrant;

    std::atomic<int> cycles{0};

    reentrant.Start([&](std::uint64_t) {
        if (++cycles == 1)

            reentrant.Request();
    });

    reentrant.Request();

    const auto deadline = std::chrono::steady_clock::now() + 5s;

    while (cycles < 2 && std::chrono::steady_clock::now() < deadline)

        std::this_thread::sleep_for(5ms);

    reentrant.Stop();

    CHECK(cycles == 2);
}

void Budget() {

    ac::logutil::LogBurstBudget budget(2, 10s);

    const auto t = ac::logutil::LogBurstBudget::Clock::time_point{};

    CHECK(budget.Admit(t).emit);

    CHECK(budget.Admit(t).emit);

    CHECK(!budget.Admit(t + 1s).emit);

    CHECK(!budget.Admit(t + 9s).emit);

    const auto next = budget.Admit(t + 10s);

    CHECK(next.emit && next.suppressed == 2);

    CHECK(budget.Admit(t + 10s).suppressed == 0);
}

void Workers() {

    namespace w = ac::workers;

    std::atomic<int> counter{0};

    for (int i = 0; i < 100; ++i)

        CHECK(w::Submit([&] { counter.fetch_add(1); }));

    auto deadline = std::chrono::steady_clock::now() + 5s;

    while (counter.load() < 100 && std::chrono::steady_clock::now() < deadline)

        std::this_thread::sleep_for(1ms);

    CHECK(counter.load() == 100);

    // A throwing job does not kill the queue: the follow-up job still runs.

    CHECK(w::Submit([] { throw std::runtime_error("boom"); }));

    CHECK(w::Submit([&] { counter.fetch_add(1); }));

    deadline = std::chrono::steady_clock::now() + 5s;

    while (counter.load() < 101 && std::chrono::steady_clock::now() < deadline)

        std::this_thread::sleep_for(1ms);

    CHECK(counter.load() == 101);

    // Named worker with stop flag: starts, observes the stop, gets joined.

    std::atomic<bool> ran{false}, sawStop{false};

    CHECK(w::StartWorker("test_worker", [&](std::atomic<bool>& stop) {
        ran.store(true);

        while (!stop.load())

            std::this_thread::sleep_for(1ms);

        sawStop.store(true);
    }));

    deadline = std::chrono::steady_clock::now() + 5s;

    while (!ran.load() && std::chrono::steady_clock::now() < deadline)

        std::this_thread::sleep_for(1ms);

    CHECK(ran.load());

    CHECK(!w::SummaryText().empty());

    // Completion is waitable independently of registry reaping/join ownership.

    w::WorkerCompletion success, failure;

    CHECK(w::StartWorker("completion", [](std::atomic<bool>&) {}, &success));

    CHECK(success.wait_for(5s) == std::future_status::ready);

    success.get();

    CHECK(w::StartWorker("throwing", [](std::atomic<bool>&) { throw std::runtime_error("worker failure"); }, &failure));

    CHECK(failure.wait_for(5s) == std::future_status::ready);

    failure.get(); // exceptions are logged by the registry, not rethrown here

    w::Shutdown();

    CHECK(sawStop.load()); // the stop flag arrived

    w::Shutdown(); // idempotente

    CHECK(!w::Submit([&] { counter.fetch_add(1); })); // rifiuto post-shutdown

    CHECK(!w::StartWorker("late", [](std::atomic<bool>&) {}));

    CHECK(!w::StartWorker("late_completion", [](std::atomic<bool>&) {}, &success));

    CHECK(!success.valid());

    CHECK(counter.load() == 101); // nessun job fantasma
}

void WorkersShutdownRace() {

    namespace w = ac::workers;

    std::atomic<bool> go{false};

    std::atomic<int> entered{0}, accepted{0}, exited{0};

    std::vector<std::thread> producers;

    for (int i = 0; i < 8; ++i)

        producers.emplace_back([&] {
            ++entered;

            while (!go.load())

                std::this_thread::yield();

            for (int j = 0; j < 100; ++j) {

                if (!w::StartWorker("racing", [&](std::atomic<bool>& stop) {
                        while (!stop.load())

                            std::this_thread::sleep_for(1ms);

                        ++exited;
                    }))

                    break;

                ++accepted;
            }
        });

    while (entered < 8)

        std::this_thread::yield();

    go = true;

    std::this_thread::sleep_for(1ms);

    w::Shutdown();

    for (auto& producer : producers)

        producer.join();

    CHECK(accepted == exited);

    CHECK(w::SummaryText().find("workers_registered=0 running=0") != std::string::npos);

    CHECK(!w::StartWorker("after_race", [](std::atomic<bool>&) {}));
}

void TtlCacheTest() {

    using ac::utils::TtlCache;

    {

        TtlCache<std::string, int> c(2, std::chrono::seconds(60));

        c.Put("a", 1);

        CHECK(c.Get("a") == 1);

        CHECK(!c.Get("missing").has_value());

        c.Put("b", 2);

        c.Put("c", 3); // overflows the cap of 2: LRU ("a") evicts

        CHECK(c.Size() == 2);

        CHECK(!c.Get("a").has_value()); // the oldest is out

        CHECK(c.Get("b") == 2);

        CHECK(c.Get("c") == 3);
    }

    {

        TtlCache<std::string, int> c(8, std::chrono::seconds(1));

        c.Put("short", 42);

        CHECK(c.Get("short") == 42);

        std::this_thread::sleep_for(std::chrono::milliseconds(1050));

        CHECK(!c.Get("short").has_value()); // expired

        CHECK(c.EvictionCount() >= 1); // expiry counts as eviction

        c.Put("fresh", 7);

        CHECK(c.Get("fresh") == 7); // a new entry comes back valid
    }

    {

        TtlCache<std::string, int> c(8, std::chrono::seconds(60));

        c.PutNegative("nope");

        CHECK(c.NegativeCount() == 1);

        CHECK(c.Get("nope") == 0); // negative hit: default value
    }
}

// --- QW8: HexCodec (roundtrip + reject deterministici) ---
