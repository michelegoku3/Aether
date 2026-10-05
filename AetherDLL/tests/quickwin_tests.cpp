#include "pch.h"
#include "core/Settings.h"
#include "core/HookManager.h"
#include "core/AbiSentinel.h"
#include "core/StructGuard.h"
#include "core/NetPacketAbi.h"
#include "utils/Strings.h"
#include "utils/DeskPaths.h"
#include "utils/CoalescingWorker.h"
#include "utils/LogBurstBudget.h"
#include "propagate/PropagationPolicy.h"
#include "core/Workers.h"
#include "hooks/wire/GamesPlayedFormat.h"
#include "core/Constants.h"
#include "hooks/wire/DonorPool.h"
#include "utils/KeyValues.h"
#include "utils/VdfText.h"
#include "utils/JsonStringField.h"
#include "network/ManifestIdentity.h"
#include "hooks/ipc/IpcReply.h"
#include "hooks/wire/UserStatsSnapshot.h"
#include "utils/TtlCache.h"
#include "credentials/HexCodec.h"
#include "utils/SignatureCodec.h"
#include "utils/IpcSpecParse.h"
#include "utils/JsonWriter.h"
#include "hooks/wire/SchemaBuckets.h"
#include "MinHook.h"
#include <atomic>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <stdexcept>
#include <thread>
#include <cstring>
#include <unordered_map>
#include <random>
#include <vector>

#define CHECK(expr) do { if (!(expr)) throw std::runtime_error("Check failed at line " + std::to_string(__LINE__) + ": " #expr); } while (false)
namespace fs = std::filesystem;
using namespace std::chrono_literals;
struct Fixture {
    fs::path dir = fs::temp_directory_path() / ("aether-quickwin-" +
        std::to_string(std::chrono::steady_clock::now().time_since_epoch().count()));
    int revision = 0;
    Fixture() { fs::create_directories(dir); }
    ~Fixture() { std::error_code ec; fs::remove_all(dir, ec); }
    fs::path Write(const char* name, const std::string& data) {
        const auto path = dir / name;
        { std::ofstream f(path, std::ios::binary); f << data; }
        fs::last_write_time(path, fs::file_time_type::clock::now() + std::chrono::seconds(++revision));
        return path;
    }
};
void Strings() {
    using namespace ac::strings;
    CHECK(EqualsIgnoreCase("EXAMPLE.com", "example.COM"));
    CHECK(!EqualsIgnoreCase("ab", "abc"));
    CHECK(Trim(" \t\r\n foo bar \t") == "foo bar");
    CHECK(Trim(" \t\r\n").empty());
    CHECK(ExtractHost("HTTPS://Example.com:443/path?q=a@b") == "Example.com");
    CHECK(ExtractHost("http://127.0.0.1/test") == "127.0.0.1");
    for (const auto* bad : {"https://u:p@host/x", "https://host:443@evil/", "https://host\\evil/",
                            "https://host:99999999999999/", "https://host:0/", "https://host:/",
                            "https://host:abc/", "https://host:1:2/", "https://[::1]/",
                            "https://host name/", "https:///x", "ftp://host/", "host/x", ""})
        CHECK(ExtractHost(bad).empty());
#ifdef _WIN32
    CHECK(Widen("caf\xc3\xa9") == L"caf\u00e9");
    CHECK(Widen("\xff").empty());
#endif
}
void Paths() {
    Fixture f;
    CHECK(ac::deskpaths::ReadDataDir(f.dir).empty());
    f.Write("desk_path.cfg", "\xEF\xBB\xBF  C:\\Desk with spaces\\AetherData \r\nignored");
    CHECK(ac::deskpaths::ReadDataDir(f.dir) == "C:\\Desk with spaces\\AetherData");
    f.Write("desk_path.cfg", "\r\n");
    CHECK(ac::deskpaths::ReadDataDir(f.dir).empty());
    f.Write("desk_path.cfg", std::string("bad\0path", 8));
    CHECK(ac::deskpaths::ReadDataDir(f.dir).empty());
    CHECK(ac::deskpaths::ConfigPath("data") == fs::path("data") / "config" / "aethercore.toml");
}
void Policy() {
    using ac::payloadprop::BlockedReason;
    CHECK(BlockedReason(L"C:\\Games\\title.exe", L"C:\\Windows") == nullptr);
    CHECK(BlockedReason(L"C:\\WindowsGames\\title.exe", L"C:\\Windows") == nullptr);
    CHECK(BlockedReason(L"C:/WINDOWS/System32/cmd.exe", L"c:\\windows\\") != nullptr);
    CHECK(BlockedReason(L"C:\\Games\\EasyAntiCheat_EOS.exe", L"C:\\Windows") != nullptr);
    CHECK(BlockedReason(L"C:\\Games\\BEService_x64.exe", L"C:\\Windows") != nullptr);
    CHECK(BlockedReason(L"C:\\Games\\CrashReportClient.exe", L"C:\\Windows") != nullptr);
    CHECK(BlockedReason(L"C:\\Games\\msedgewebview2.exe", L"C:\\Windows") != nullptr);
    CHECK(BlockedReason(L"title.exe", L"C:\\Windows") != nullptr);
    CHECK(BlockedReason(L"", L"C:\\Windows") != nullptr);
    CHECK(BlockedReason(L"C:\\Games\\title.exe", L"") != nullptr);
}
void Worker() {
    ac::utils::CoalescingWorker worker;
    std::atomic<std::uint64_t> requests{0}, callbacks{0};
    std::atomic<int> active{0}, maxActive{0};
    worker.Start([&](std::uint64_t n) {
        const auto running = ++active;
        if (running > maxActive) maxActive = running;
        requests += n;
        ++callbacks;
        --active;
    });
    std::vector<std::thread> threads;
    for (int i = 0; i < 8; ++i) threads.emplace_back([&] { for (int j = 0; j < 1000; ++j) worker.Request(); });
    for (auto& t : threads) t.join();
    worker.Stop(); // drains pending requests, including when no timer fired yet
    CHECK(requests == 8000);
    CHECK(callbacks > 0 && callbacks < 8000);
    CHECK(maxActive == 1);
    worker.Request(); worker.Stop();
    CHECK(requests == 8000);
    // Requests arriving DURING serialization must get another write.
    ac::utils::CoalescingWorker reentrant;
    std::atomic<int> cycles{0};
    reentrant.Start([&](std::uint64_t) { if (++cycles == 1) reentrant.Request(); });
    reentrant.Request();
    const auto deadline = std::chrono::steady_clock::now() + 5s;
    while (cycles < 2 && std::chrono::steady_clock::now() < deadline) std::this_thread::sleep_for(5ms);
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
void Settings() {
    Fixture f;
    auto path = f.Write("settings.toml", "[presence]\ncustom_game_name='old'\nshowonline_apps=[1,2]\n");
    ac::Settings::Initialize(path.string());
    const auto old = ac::Settings::Snapshot();
    CHECK(old->presenceCustomGameName == "old");
    f.Write("settings.toml", "[presence]\ncustom_game_name='new'\nshowonline_apps=[3]\n");
    ac::Settings::ReloadIfModified(path.string());
    auto current = ac::Settings::Snapshot();
    CHECK(current->presenceCustomGameName == "new");
    CHECK(current->presenceShowOnlineApps == std::vector<std::uint32_t>{3});
    CHECK(old->presenceCustomGameName == "old" && old->presenceShowOnlineApps.size() == 2);
    ac::Settings::ReloadIfModified(path.string());
    CHECK(ac::Settings::Snapshot() == current);
    f.Write("settings.toml", "[presence\ninvalid");
    ac::Settings::ReloadIfModified(path.string());
    CHECK(ac::Settings::Snapshot() == current);
    fs::remove(path);
    ac::Settings::ReloadIfModified(path.string());
    CHECK(ac::Settings::Snapshot() == current);
    std::atomic<bool> stop{false}, coherent{true};
    std::thread reader([&] {
        while (!stop) {
            const auto snap = ac::Settings::Snapshot();
            if (snap->presenceShowOnlineApps.empty()) coherent = false;
            // Exercise references while the publisher replaces large allocations.
            const auto& name = snap->presenceCustomGameName;
            std::this_thread::yield();
            if (name.empty()) coherent = false;
        }
    });
    for (int i = 0; i < 100; ++i) {
        f.Write("settings.toml", "[presence]\ncustom_game_name='revision" + std::to_string(i) + "'\nshowonline_apps=[4,5,6]\n");
        ac::Settings::ReloadIfModified(path.string());
    }
    stop = true; reader.join();
    CHECK(coherent);
    CHECK(ac::Settings::Snapshot()->presenceCustomGameName == "revision99");

    // Startup-only injection selection is strict, with auto as the default.
    bool valid = false;
    CHECK(ac::Settings::Load("nonexistent.toml", &valid).diversionMode == ac::DiversionMode::Auto);
    CHECK(!valid);
    auto modePath = f.Write("injection.toml", "[injection]\ndiversion_mode = 'copy'\n");
    CHECK(ac::Settings::Load(modePath.string(), &valid).diversionMode == ac::DiversionMode::Copy);
    CHECK(valid);
    f.Write("injection.toml", "[injection]\ndiversion_mode = 'live'\n");
    CHECK(ac::Settings::Load(modePath.string(), &valid).diversionMode == ac::DiversionMode::Live);
    CHECK(valid);
    f.Write("injection.toml", "[injection]\ndiversion_mode = 'invalid'\n");
    ac::Settings::Load(modePath.string(), &valid);
    CHECK(!valid);
}
void Workers() {
    namespace w = ac::workers;
    std::atomic<int> counter{0};
    for (int i = 0; i < 100; ++i) CHECK(w::Submit([&] { counter.fetch_add(1); }));
    auto deadline = std::chrono::steady_clock::now() + 5s;
    while (counter.load() < 100 && std::chrono::steady_clock::now() < deadline)
        std::this_thread::sleep_for(1ms);
    CHECK(counter.load() == 100);
    // Un job che lancia non uccide la coda: il job successivo gira comunque.
    CHECK(w::Submit([] { throw std::runtime_error("boom"); }));
    CHECK(w::Submit([&] { counter.fetch_add(1); }));
    deadline = std::chrono::steady_clock::now() + 5s;
    while (counter.load() < 101 && std::chrono::steady_clock::now() < deadline)
        std::this_thread::sleep_for(1ms);
    CHECK(counter.load() == 101);
    // Worker nominato con stop flag: parte, osserva lo stop, viene joinato.
    std::atomic<bool> ran{false}, sawStop{false};
    CHECK(w::StartWorker("test_worker", [&](std::atomic<bool>& stop) {
        ran.store(true);
        while (!stop.load()) std::this_thread::sleep_for(1ms);
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
    CHECK(w::StartWorker("throwing", [](std::atomic<bool>&) {
        throw std::runtime_error("worker failure");
    }, &failure));
    CHECK(failure.wait_for(5s) == std::future_status::ready);
    failure.get(); // exceptions are logged by the registry, not rethrown here
    w::Shutdown();
    CHECK(sawStop.load());              // lo stop flag è arrivato
    w::Shutdown();                      // idempotente
    CHECK(!w::Submit([&] { counter.fetch_add(1); }));   // rifiuto post-shutdown
    CHECK(!w::StartWorker("late", [](std::atomic<bool>&) {}));
    CHECK(!w::StartWorker("late_completion", [](std::atomic<bool>&) {}, &success));
    CHECK(!success.valid());
    CHECK(counter.load() == 101);       // nessun job fantasma
}

void WorkersShutdownRace() {
    namespace w = ac::workers;
    std::atomic<bool> go{false};
    std::atomic<int> entered{0}, accepted{0}, exited{0};
    std::vector<std::thread> producers;
    for (int i = 0; i < 8; ++i) producers.emplace_back([&] {
        ++entered;
        while (!go.load()) std::this_thread::yield();
        for (int j = 0; j < 100; ++j) {
            if (!w::StartWorker("racing", [&](std::atomic<bool>& stop) {
                while (!stop.load()) std::this_thread::sleep_for(1ms);
                ++exited;
            })) break;
            ++accepted;
        }
    });
    while (entered < 8) std::this_thread::yield();
    go = true;
    std::this_thread::sleep_for(1ms);
    w::Shutdown();
    for (auto& producer : producers) producer.join();
    CHECK(accepted == exited);
    CHECK(w::SummaryText().find("workers_registered=0 running=0") != std::string::npos);
    CHECK(!w::StartWorker("after_race", [](std::atomic<bool>&) {}));
}

void SchemaBuckets() {
    auto fixture = [](const std::string& bucket, const std::string& child,
                      bool entry, const std::string& section = "stats") {
        std::vector<std::uint8_t> data;
        auto dict = [&](const std::string& name) {
            data.push_back(0); data.insert(data.end(), name.begin(), name.end()); data.push_back(0);
        };
        dict("123"); dict(section); dict(bucket); dict(child);
        if (entry) { dict("0"); data.push_back(8); }
        for (int i = 0; i < 5; ++i) data.push_back(8);
        return data;
    };
    auto parse = [](const std::vector<std::uint8_t>& data) {
        ac::backup::statscache::detail::SchemaBucketWalker walker;
        CHECK(ac::kv1::WalkBinary(data.data(), data.size(), walker));
        return walker.buckets;
    };
    CHECK(parse(fixture("0", "bits", true)).count(0) == 1);
    CHECK(parse(fixture("200", "bits", true)).count(200) == 1);
    CHECK(parse(fixture("200", "bits", false)).empty());
    CHECK(parse(fixture("200", "display", true)).empty());
    CHECK(parse(fixture("200", "bits", true, "other")).empty());
    for (const auto* bad : {"", "no", "-1", "1x", "4294967296"})
        CHECK(parse(fixture(bad, "bits", true)).empty());
    auto truncated = fixture("200", "bits", true);
    truncated.resize(6); // truncated key; no completed bucket
    ac::backup::statscache::detail::SchemaBucketWalker walker;
    CHECK(!ac::kv1::WalkBinary(truncated.data(), truncated.size(), walker));
    CHECK(walker.buckets.empty());
}

void Registry() {
    ac::HookManager manager;
    manager.RecordMissed("late", ac::MissReason::PatternUnresolved);
    auto before = manager.Snapshot();
    CHECK(before.missed.size() == 1);
    manager.RegisterHook("late", nullptr, nullptr, nullptr);
    CHECK(manager.InstallAll());
    CHECK(manager.Snapshot().installed.size() == 1 && manager.Snapshot().missed.empty());
    CHECK(before.missed.size() == 1); // owned immutable copy
    std::atomic<bool> stop{false}, coherent{true};
    std::thread reader([&] {
        while (!stop) if (manager.Snapshot().installed.empty()) coherent = false;
    });
    for (int i = 0; i < 1000; ++i) {
        const auto name = "hook" + std::to_string(i);
        manager.RecordMissed(name, ac::MissReason::PatternUnresolved);
        manager.RegisterHook(name, nullptr, nullptr, nullptr);
    }
    CHECK(manager.InstallAll());
    stop = true; reader.join();
    CHECK(coherent);
    CHECK(manager.Snapshot().installed.size() == 1001);
    CHECK(manager.InstallAll());
    CHECK(manager.Snapshot().installed.size() == 1001);
    manager.UninstallAll(); manager.UninstallAll();
    CHECK(manager.Snapshot().installed.empty());

    ac::HookManager retried;
    retried.RegisterHook("LoadModuleWithPath", nullptr, nullptr, nullptr);
    testCreateFails = true;
    CHECK(retried.InstallAll());
    CHECK(retried.InstallAll());
    CHECK(retried.Snapshot().missed.size() == 1);  // no duplicate retry misses
    testCreateFails = false;
    CHECK(retried.InstallAll());
    CHECK(retried.Snapshot().installed.size() == 1);
    CHECK(retried.Snapshot().missed.empty());
}

// ---------------------------------------------------------------------------
// CNetPacket ABI resolver (core/NetPacketAbi.h).
//
// Regression fence for the beta-1790380355 crash: the layout must be
// identified from live packets, must NEVER latch on garbage, and must
// disable itself instead of falling back to a compiled default.
// ---------------------------------------------------------------------------
namespace netpkt_test {
using namespace ac::abi::netpkt;

// A synthetic CNetPacket: 0x40 bytes of object plus a frame buffer. `dataOff`
// selects which layout the fake object is built in, so the same code models
// both the stable (0x08) and the beta (0x10) client.
struct FakePacket {
    alignas(16) std::uint8_t obj[0x40]{};
    std::uint8_t frame[256]{};

    void Build(std::uint32_t dataOff, std::uint32_t headerLen, std::int32_t ref = 1,
               std::uint32_t protoFlag = ac::abi::netpkt::kProtoFlag,
               std::uint32_t frameLen = 0) {
        std::memset(obj, 0, sizeof(obj));
        std::memset(frame, 0, sizeof(frame));
        const std::uint32_t len = frameLen ? frameLen : (8 + headerLen);
        const std::uint32_t rawEMsg = protoFlag | 0x2BFu;   // a real Steam EMsg
        std::memcpy(frame, &rawEMsg, 4);
        std::memcpy(frame + 4, &headerLen, 4);
        std::uint8_t* ptr = frame;
        std::memcpy(obj + dataOff, &ptr, sizeof(ptr));
        std::memcpy(obj + SizeOffFor(dataOff), &len, 4);
        std::memcpy(obj + RefOffFor(dataOff), &ref, 4);
    }

    const void* Packet() const { return obj; }
};

// Readability oracle for the tests: only the fake object and its frame are
// readable, everything else is "unmapped". Stricter than Windows, which is
// exactly what we want — a probe that needs real memory to be wrong fails here.
struct Arena { const FakePacket* p; };
bool ArenaReadable(const void* addr, std::size_t bytes, void* ctx) {
    const auto* a = static_cast<const Arena*>(ctx);
    const auto start = reinterpret_cast<std::uintptr_t>(addr);
    const auto objLo = reinterpret_cast<std::uintptr_t>(a->p->obj);
    const auto frameLo = reinterpret_cast<std::uintptr_t>(a->p->frame);
    const bool inObj = start >= objLo && start + bytes <= objLo + sizeof(a->p->obj);
    const bool inFrame = start >= frameLo && start + bytes <= frameLo + sizeof(a->p->frame);
    return inObj || inFrame;
}

// Drives `r` with the same packet until it latches or gives up.
Resolver::Step Feed(Resolver& r, const FakePacket& pkt, int times) {
    Arena arena{&pkt};
    Resolver::Step last = Resolver::Step::NoEvidence;
    for (int i = 0; i < times; ++i) last = r.Observe(pkt.Packet(), &ArenaReadable, &arena);
    return last;
}
}  // namespace netpkt_test

void NetPacket() {
    using namespace ac::abi::netpkt;
    using netpkt_test::FakePacket;
    using netpkt_test::Feed;

    // 1. Golden values: the layouts proven by disassembly must not drift.
    CHECK(kLayouts[0].dataOff == 0x08 && SizeOffFor(0x08) == 0x10 && RefOffFor(0x08) == 0x14);
    CHECK(kLayouts[1].dataOff == 0x10 && SizeOffFor(0x10) == 0x18 && RefOffFor(0x10) == 0x1C);

    // 2. A stable-layout packet latches on the SECOND agreeing packet, never
    //    the first (one packet is not proof).
    {
        FakePacket pkt; pkt.Build(0x08, 32);
        Resolver r;
        CHECK(Feed(r, pkt, 1) == Resolver::Step::AwaitingConfirm);
        CHECK(!r.IsResolved());
        CHECK(Feed(r, pkt, 1) == Resolver::Step::Latched);
        CHECK(r.IsResolved() && r.DataOffset() == 0x08);
        CHECK(std::string(r.LayoutName()) == "stable");
        CHECK(r.Confirmations() == 2);   // no hint: two packets had to agree
    }

    // 3. Same code, beta layout -> the OTHER offset. This is the case that
    //    crashed Steam when it was compiled in.
    {
        FakePacket pkt; pkt.Build(0x10, 64);
        Resolver r;
        Feed(r, pkt, 2);
        CHECK(r.IsResolved() && r.DataOffset() == 0x10);
        CHECK(std::string(r.LayoutName()) == "beta");
    }

    // 4. A layout we do not know (future Valve shift) must DISABLE, never
    //    guess: no field is touched for the rest of the session.
    {
        FakePacket pkt; pkt.Build(0x18, 32);          // hypothetical +0x18
        Resolver r;
        Feed(r, pkt, kMaxProbeAttempts + 2);
        CHECK(r.IsDisabled());
        CHECK(!r.IsResolved() && r.DataOffset() == 0);
        CHECK(std::string(r.LayoutName()) == "disabled");
    }

    // 5. Hostile / random memory must never latch anything.
    {
        FakePacket pkt;
        std::mt19937 rng(1234);
        Resolver r;
        netpkt_test::Arena arena{&pkt};
        for (int i = 0; i < 400; ++i) {
            for (auto& b : pkt.obj) b = static_cast<std::uint8_t>(rng() & 0xFF);
            for (auto& b : pkt.frame) b = static_cast<std::uint8_t>(rng() & 0xFF);
            r.Observe(pkt.Packet(), &netpkt_test::ArenaReadable, &arena);
            CHECK(!r.IsResolved());
        }
    }

    // 6. Valve's own predicate is enforced: a non-proto frame, a bogus
    //    refcount and an out-of-range headerLength are all rejected.
    {
        netpkt_test::Arena a{nullptr};
        FakePacket nonProto; nonProto.Build(0x08, 32, 1, /*protoFlag=*/0);
        a.p = &nonProto;
        CHECK(!CandidateMatches(nonProto.Packet(), 0x08, &netpkt_test::ArenaReadable, &a));

        FakePacket freed; freed.Build(0x08, 32, /*ref=*/0);
        a.p = &freed;
        CHECK(!CandidateMatches(freed.Packet(), 0x08, &netpkt_test::ArenaReadable, &a));

        FakePacket liar; liar.Build(0x08, /*headerLen=*/900, 1,
                                    ac::abi::netpkt::kProtoFlag, /*frameLen=*/64);
        a.p = &liar;   // headerLen > len - 8
        CHECK(!CandidateMatches(liar.Packet(), 0x08, &netpkt_test::ArenaReadable, &a));

        FakePacket good; good.Build(0x08, 32);
        a.p = &good;
        CHECK(CandidateMatches(good.Packet(), 0x08, &netpkt_test::ArenaReadable, &a));
    }

    // 7. A poisoned hint (what a wrong ABI table would provide) does not
    //    override the probe: the true layout still wins.
    {
        FakePacket pkt; pkt.Build(0x10, 32);
        Resolver r;
        r.Hint(0x08);                       // table says stable, packet says beta
        Feed(r, pkt, 3);
        CHECK(r.IsResolved() && r.DataOffset() == 0x10);
        CHECK(r.Confirmations() == 2);   // the wrong hint bought nothing
    }

    // 9b. Write barrier: once latched, a matching packet may be written, a
    //     packet that no longer matches may not, and eight consecutive
    //     rejections shut the feature down for good.
    {
        FakePacket good; good.Build(0x10, 32);
        netpkt_test::Arena arena{&good};
        Resolver r;
        r.Hint(0x10);
        Feed(r, good, 1);
        CHECK(r.IsResolved());
        CHECK(r.BeginWrite(good.Packet(), &netpkt_test::ArenaReadable, &arena));
        CHECK(r.WriteRejects() == 0);

        // A legacy (non-protobuf) packet is now WRITABLE: identification
        // needs the protobuf flag, the write gate does not.
        FakePacket legacy; legacy.Build(0x10, 32, 1, /*protoFlag=*/0);
        netpkt_test::Arena legacyArena{&legacy};
        CHECK(!CandidateMatches(legacy.Packet(), 0x10, &netpkt_test::ArenaReadable, &legacyArena));
        CHECK(r.BeginWrite(legacy.Packet(), &netpkt_test::ArenaReadable, &legacyArena));

        FakePacket bad; bad.Build(0x08, 32);      // stable-shaped object, beta layout latched
        netpkt_test::Arena badArena{&bad};
        for (int i = 0; i < kMaxWriteMismatches - 1; ++i) {
            CHECK(!r.BeginWrite(bad.Packet(), &netpkt_test::ArenaReadable, &badArena));
            CHECK(!r.IsDisabled());               // one odd packet is not a verdict
        }
        CHECK(!r.BeginWrite(bad.Packet(), &netpkt_test::ArenaReadable, &badArena));
        CHECK(r.IsDisabled());                    // ...eight in a row is
        CHECK(r.WriteRejects() == kMaxWriteMismatches);
        // Terminal: even a good packet is refused afterwards.
        CHECK(!r.BeginWrite(good.Packet(), &netpkt_test::ArenaReadable, &arena));
    }

    // 9c. A single mismatch does not arm the shutdown: the counter resets as
    //     soon as a matching packet arrives (login traffic is mixed).
    {
        FakePacket good; good.Build(0x10, 32);
        FakePacket bad; bad.Build(0x08, 32);
        netpkt_test::Arena ga{&good}, ba{&bad};
        Resolver r; r.Hint(0x10); Feed(r, good, 1);
        for (int i = 0; i < 50; ++i) {
            CHECK(!r.BeginWrite(bad.Packet(), &netpkt_test::ArenaReadable, &ba));
            CHECK(r.BeginWrite(good.Packet(), &netpkt_test::ArenaReadable, &ga));
        }
        CHECK(!r.IsDisabled());
    }

    // 9. A correct hint latches on the FIRST live packet, and says so
    //    (Confirmations()==1 is what the log line reports).
    {
        FakePacket pkt; pkt.Build(0x10, 32);
        Resolver r;
        r.Hint(0x10);
        CHECK(Feed(r, pkt, 1) == Resolver::Step::Latched);
        CHECK(r.DataOffset() == 0x10 && r.Attempts() == 1 && r.Confirmations() == 1);
    }


    // 10. Phase 4: a layout this binary was never compiled with, proposed by
    //     the per-build ABI table, is latched only after the probe agrees.
    {
        FakePacket pkt; pkt.Build(0x20, 32);          // a future Valve shift
        netpkt_test::Arena arena{&pkt};
        Resolver r;
        CHECK(!CandidateMatches(pkt.Packet(), 0x08, &netpkt_test::ArenaReadable, &arena));
        // Without the table the layout is unknown and stays unknown.
        CHECK(Feed(r, pkt, 4) == Resolver::Step::NoEvidence);
        CHECK(!r.IsResolved());

        // The table proposes it; the probe confirms it on a live packet.
        Resolver withTable;
        CHECK(withTable.AddCandidate(0x20));
        withTable.Hint(0x20);
        CHECK(Feed(withTable, pkt, 1) == Resolver::Step::Latched);
        CHECK(withTable.DataOffset() == 0x20);
        CHECK(std::string(withTable.LayoutName()) == "from-abi-table");
    }

    // 11. A table that proposes the WRONG layout still loses to the packet.
    {
        FakePacket pkt; pkt.Build(0x10, 48);
        Resolver r;
        CHECK(r.AddCandidate(0x20));
        r.Hint(0x20);
        Feed(r, pkt, 3);
        CHECK(r.IsResolved() && r.DataOffset() == 0x10);
    }

    // 12. Candidate registration is idempotent and bounded.
    {
        Resolver r;
        CHECK(r.AddCandidate(0x08));                  // already known: accepted, not stored
        CHECK(r.AddCandidate(0x20));
        CHECK(r.AddCandidate(0x20));                  // idempotent
        for (int i = 0; i < kMaxExtraLayouts; ++i) r.AddCandidate(0x100 + 8 * i);
        CHECK(!r.AddCandidate(0x900));                // full: refused, not silently dropped
    }

    // 8. Accessors are inert while unresolved: writing through them must not
    //    touch the object (they return the trash sink).
    {
        Global().Reset();
        CHECK(!Global().IsResolved());
        FakePacket pkt; pkt.Build(0x10, 32);
        auto* opaque = reinterpret_cast<ac::steam::CNetPacket*>(pkt.obj);
        std::uint8_t sentinel[4]{1, 2, 3, 4};
        std::uint8_t before[sizeof(pkt.obj)];
        std::memcpy(before, pkt.obj, sizeof(before));
        Data(opaque) = sentinel;
        Size(opaque) = 0xDEADBEEF;
        CHECK(std::memcmp(before, pkt.obj, sizeof(before)) == 0);
        Global().Reset();
    }
}


// ---------------------------------------------------------------------------
// ABI sentinel (core/AbiSentinel.h).
//
// The byte sequences below are REAL: taken from steamclient64.dll build
// 1790380355 (beta). They are the regression fence for the class of defect
// that a signature check cannot see — a pattern table whose RVA points into
// the middle of a function.
// ---------------------------------------------------------------------------
void Sentinel() {
    using namespace ac::abi::sentinel;

    // A slice of the real .pdata: RecvPkt's first fragment, the fragment that
    // contains the bad 0x5BC460 pin, and CNetPacket::AddRef.
    const PdataEntry pdata[] = {
        {0x5BC0F0, 0x5BC197},   // CCMConnection::RecvPkt
        {0x5BC1A0, 0x5BC1B2},   // CCMInterface::RecvPkt (hot fragment)
        {0x5BC32A, 0x5BD139},   // ...cold fragment: 0x5BC460 lives in here
        {0xE82730, 0xE827DA},   // CNetPacket::AddRef
    };
    const std::size_t n = sizeof(pdata) / sizeof(pdata[0]);

    // 1. Real entry point, listed in .pdata -> accepted.
    //    0x5BC1A0: preceded by int3 padding, starts with `mov rax,rsp`.
    {
        const std::uint8_t first[] = {0x48, 0x8B, 0xC4, 0x55, 0x48, 0x8D, 0xA8, 0xE8};
        const Verdict v = Classify(0x5BC1A0, true, pdata, n, 0xCC, first, sizeof(first));
        CHECK(v == Verdict::Ok);
        CHECK(Accepted(v));
    }

    // 2. THE regression: 0x5BC460 is inside the cold fragment. Bytes look
    //    like a perfectly normal instruction and the previous byte is live
    //    code (`48 8B D3`). Only .pdata can tell, and it does.
    {
        const std::uint8_t first[] = {0x48, 0x89, 0x44, 0x24, 0x30, 0x48, 0x8D, 0x4C};
        const Verdict v = Classify(0x5BC460, true, pdata, n, 0xD3, first, sizeof(first));
        CHECK(v == Verdict::NotFunctionStart);
        CHECK(!Accepted(v));
    }

    // 3. Leaf function with no unwind entry (GetPipeClient, 0x89F270, starts
    //    with `test edx,edx`): .pdata cannot help, the int3 padding before it
    //    can. This must NOT be rejected — Aether hooks it.
    {
        const std::uint8_t first[] = {0x85, 0xD2, 0x74, 0x30, 0x44, 0x0F, 0xB7, 0xCA};
        const Verdict v = Classify(0x89F270, true, pdata, n, 0xCC, first, sizeof(first));
        CHECK(v == Verdict::OkLeaf);
        CHECK(Accepted(v));
    }

    // 4. A function that starts right after a `ret` (no padding) is still a
    //    function: CNetPacket's mini-ctor at 0xE826D0.
    {
        const std::uint8_t first[] = {0x33, 0xC0, 0x89, 0x41, 0x1C, 0x48, 0x89, 0x41};
        CHECK(Classify(0xE826D0, true, pdata, n, 0xC3, first, sizeof(first)) == Verdict::OkLeaf);
    }

    // 5. An address in a .pdata gap preceded by live code: unprovable, so
    //    refused. Better a skipped hook than a jump into a basic block.
    {
        const std::uint8_t first[] = {0x8B, 0x41, 0x10, 0xC3, 0xCC, 0xCC, 0xCC, 0xCC};
        CHECK(Classify(0x700000, true, pdata, n, 0x48, first, sizeof(first)) ==
              Verdict::NoPredecessorBoundary);
    }

    // 6. Padding, outside-code and unreadable targets.
    {
        const std::uint8_t pad[] = {0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC};
        CHECK(Classify(0x5BC198, true, pdata, n, 0xC3, pad, sizeof(pad)) ==
              Verdict::PaddingAtTarget);
        const std::uint8_t zero[8] = {};
        CHECK(Classify(0x5BC198, true, pdata, n, 0xC3, zero, sizeof(zero)) ==
              Verdict::PaddingAtTarget);
        const std::uint8_t first[] = {0x48, 0x8B, 0xC4};
        CHECK(Classify(0x1400000, false, pdata, n, 0xCC, first, sizeof(first)) ==
              Verdict::OutsideCode);
        CHECK(Classify(0x5BC1A0, true, pdata, n, 0xCC, nullptr, 0) == Verdict::NoBytes);
    }

    // 7. No .pdata at all (stripped module): the classifier still works off
    //    the predecessor byte instead of giving up.
    {
        const std::uint8_t first[] = {0x40, 0x53, 0x48, 0x83, 0xEC, 0x30, 0xFF, 0x41};
        CHECK(Classify(0xE82730, true, nullptr, 0, 0xCC, first, sizeof(first)) ==
              Verdict::OkLeaf);
        CHECK(Classify(0xE82730, true, nullptr, 0, 0x8B, first, sizeof(first)) ==
              Verdict::NoPredecessorBoundary);
    }

    // 8. Every verdict has text, and only the two positive ones are accepted.
    {
        const Verdict all[] = {Verdict::Ok, Verdict::OkLeaf, Verdict::NotFunctionStart,
                               Verdict::NoPredecessorBoundary, Verdict::PaddingAtTarget,
                               Verdict::OutsideCode, Verdict::NoBytes};
        int accepted = 0;
        for (const Verdict v : all) {
            CHECK(VerdictText(v) != nullptr && VerdictText(v)[0] != '\0');
            if (Accepted(v)) ++accepted;
        }
        CHECK(accepted == 2);
    }
}


// ---------------------------------------------------------------------------
// Struct guards (core/StructGuard.h) — PackageInfo and AppOwnership.
//
// The layouts are proven by disassembly of both shipped builds; these tests
// fence the invariants that let us refuse a write instead of corrupting Steam.
// ---------------------------------------------------------------------------
void StructGuard() {
    using namespace ac::abi::guard;
    using ac::steam::AppOwnership;
    using ac::steam::PackageInfo;
    using ac::steam::AppId;

    // 1. The offsets ARE the disassembly. (static_assert already fences them
    //    at compile time; this keeps the numbers visible in a test too.)
    CHECK(offsetof(AppOwnership, ownsLicense) == 0x24);
    CHECK(offsetof(AppOwnership, familyShared) == 0x35);
    CHECK(offsetof(PackageInfo, appIdVec) == 0x40);
    CHECK(offsetof(PackageInfo, depotIdVec) == 0x58);
    CHECK(sizeof(ac::steam::CUtlVector<AppId>) == 0x18);

    // 2. A well-formed ownership record passes.
    AppOwnership own{};
    own.releaseState = ac::steam::AppReleaseState::Released;
    own.existInPackageNums = 2;
    own.ownsLicense = true;
    own.familyShared = false;
    CHECK(Passed(CheckOwnership(&own)));
    CHECK(CheckOwnership(nullptr) == Reason::NullObject);

    // 3. The decisive test: ONE byte of the bool block holding something that
    //    is not 0/1 — what a shifted layout looks like — is refused.
    for (std::size_t i = 0; i < kOwnershipBoolCount; ++i) {
        AppOwnership shifted{};
        shifted.releaseState = ac::steam::AppReleaseState::Released;
        reinterpret_cast<std::uint8_t*>(&shifted)[kOwnershipBoolFirst + i] = 0x7F;
        CHECK(CheckOwnership(&shifted) == Reason::BoolBlockNotBoolean);
    }

    // 4. A whole struct filled with pointer-ish garbage (the +8 shift class of
    //    bug) never passes.
    {
        AppOwnership garbage{};
        std::mt19937 rng(99);
        for (std::size_t i = 0; i < sizeof(garbage); ++i) {
            reinterpret_cast<std::uint8_t*>(&garbage)[i] = static_cast<std::uint8_t>(rng() | 2);
        }
        CHECK(!Passed(CheckOwnership(&garbage)));
    }

    // 5. Out-of-range scalars.
    {
        AppOwnership bad{};
        bad.releaseState = static_cast<ac::steam::AppReleaseState>(99);
        CHECK(CheckOwnership(&bad) == Reason::ReleaseStateOutOfRange);
        AppOwnership bad2{};
        bad2.existInPackageNums = 999999;
        CHECK(CheckOwnership(&bad2) == Reason::PackageCountAbsurd);
    }

    // 6. PackageInfo vectors: the invariant that protects the heap.
    {
        std::vector<AppId> storage(64, 0);
        PackageInfo pkg{};
        pkg.appIdVec.mem.memory = storage.data();
        pkg.appIdVec.mem.allocationCount = 64;
        pkg.appIdVec.size = 10;
        CHECK(Passed(CheckPackage(&pkg)));

        pkg.appIdVec.size = 65;                      // size > alloc: would write past the end
        CHECK(CheckPackage(&pkg) == Reason::VectorSizeExceedsAlloc);

        pkg.appIdVec.size = 10;
        pkg.appIdVec.mem.memory = nullptr;           // elements but no storage
        CHECK(CheckPackage(&pkg) == Reason::VectorMemoryNull);

        pkg.appIdVec.mem.memory = storage.data();
        pkg.appIdVec.mem.allocationCount = 0xFFFFFFF;  // absurd capacity
        CHECK(CheckPackage(&pkg) == Reason::VectorAllocAbsurd);

        pkg.appIdVec.mem.allocationCount = 64;
        pkg.appIdVec.mem.memory =
            reinterpret_cast<AppId*>(reinterpret_cast<std::uint8_t*>(storage.data()) + 1);
        CHECK(CheckPackage(&pkg) == Reason::VectorMemoryMisaligned);

        // An empty, zeroed vector is legitimate (fresh package).
        PackageInfo fresh{};
        CHECK(Passed(CheckPackage(&fresh)));

        // The second vector is checked too, not just the first (the checks
        // fire in order, so size-vs-alloc is reported before the null test).
        PackageInfo second{};
        second.depotIdVec.size = 5;
        CHECK(CheckPackage(&second) == Reason::VectorSizeExceedsAlloc);
        second.depotIdVec.mem.allocationCount = 8;   // capacity claimed, storage absent
        CHECK(CheckPackage(&second) == Reason::VectorMemoryNull);
        CHECK(CheckPackage(nullptr) == Reason::NullObject);
    }

    // 8. Phase 4: the published table is checked against the compiled layout.
    {
        ResetCounters();
        std::unordered_map<std::string, std::uint32_t> table = {
            {"AppOwnership.bool_block_start", 0x24},
            {"AppOwnership.size_bytes", 0x36},
            {"PackageInfo.appIdVec", 0x40},
            {"PackageInfo.depotIdVec", 0x58},
            {"CUtlVector.count", 0x10},
        };
        CHECK(ApplyTable(table, "test") == 5);
        CHECK(!LayoutContradicted());
        AppOwnership fine{};
        fine.releaseState = ac::steam::AppReleaseState::Released;
        CHECK(Passed(CheckOwnership(&fine)));

        // A table describing a layout this binary was not compiled with must
        // stop every guarded write: a C++ struct cannot be re-laid-out at
        // runtime, so "adapting" would mean writing to the wrong offsets.
        table["AppOwnership.bool_block_start"] = 0x2C;
        ApplyTable(table, "test");
        CHECK(LayoutContradicted());
        CHECK(CheckOwnership(&fine) == Reason::TableMismatch);
        PackageInfo pkg{};
        CHECK(CheckPackage(&pkg) == Reason::TableMismatch);

        // Fields the table does not carry are not an opinion (older tables).
        ResetCounters();
        CHECK(ApplyTable({{"Unrelated.field", 1}}, "test") == 0);
        CHECK(!LayoutContradicted());
    }

    // 9. DepotEntry: the table Aether rewrites manifest ids into.
    {
        ResetCounters();
        using ac::steam::DepotEntry;
        CHECK(sizeof(DepotEntry) == 0x20);              // stride proven by the loops
        CHECK(offsetof(DepotEntry, manifestGid) == 0x08);
        CHECK(offsetof(DepotEntry, manifestSize) == 0x10);
        CHECK(offsetof(ac::steam::AppOwnership, borrowed) == 0x2F);   // not 0x31

        std::vector<DepotEntry> rows(4);
        rows[0] = DepotEntry{1234, 10, 0xAABBCCDD11223344ull, 50ull * 1024 * 1024, 0, 1, 0, 1, 0};
        ac::steam::CUtlVector<DepotEntry> vec{};
        vec.mem.memory = rows.data();
        vec.mem.allocationCount = 4;
        vec.size = 2;                                    // row 1 is a zeroed tail row
        CHECK(Passed(CheckDepotVector(&vec)));

        rows[1].depotId = 0;                             // id missing but data present
        rows[1].manifestGid = 0x1234;
        CHECK(CheckDepotVector(&vec) == Reason::DepotEntryImplausible);

        rows[1] = DepotEntry{7, 7, 1, 1, 0, 2, 0, 0, 0}; // a "bool" holding 2
        CHECK(CheckDepotVector(&vec) == Reason::DepotEntryImplausible);

        rows[1] = DepotEntry{7, 7, 1, 1ull << 60, 0, 0, 0, 0, 0};  // absurd size
        CHECK(CheckDepotVector(&vec) == Reason::DepotEntryImplausible);

        rows[1] = DepotEntry{7, 7, 99, 1024, 0, 0, 0, 0, 0};
        CHECK(Passed(CheckDepotVector(&vec)));
        CHECK(CheckDepotVector(nullptr) == Reason::NullObject);

        vec.size = 9;                                    // size beyond capacity
        CHECK(CheckDepotVector(&vec) == Reason::VectorSizeExceedsAlloc);
    }

    // 7. Every reason has text.
    {
        const Reason all[] = {Reason::Ok, Reason::NullObject, Reason::BoolBlockNotBoolean,
                              Reason::ReleaseStateOutOfRange, Reason::PackageCountAbsurd,
                              Reason::VectorSizeExceedsAlloc, Reason::VectorMemoryNull,
                              Reason::VectorMemoryMisaligned, Reason::VectorAllocAbsurd,
                              Reason::TableMismatch, Reason::DepotEntryImplausible};
        for (const Reason r : all) CHECK(ReasonText(r) != nullptr && ReasonText(r)[0] != '\0');
    }
}


void GamesPlayedFormatTest() {
    namespace fmt = ac::hooks::GamesPlayedFormat;
    // AppIdFromGameId: low-24 mask.
    CHECK(fmt::AppIdFromGameId(0x020000000000ull | 1687950u) == 1687950u);
    CHECK(fmt::AppIdFromGameId(480u) == 480u);
    // ImageStem: path strip, Unreal suffix, trailing "-steam".
    CHECK(fmt::ImageStem("C:\\games\\Bodycam-Win64-Shipping.exe") == "Bodycam");
    CHECK(fmt::ImageStem("ReadyOrNotSteam-Win64-Shipping.exe") == "ReadyOrNot");
    CHECK(fmt::ImageStem("P5R.exe") == "P5R");
    CHECK(fmt::ImageStem("") == "");
    // NameIsUsable: rejects empty and the Spacewar fold.
    CHECK(!fmt::NameIsUsable(""));
    CHECK(!fmt::NameIsUsable("Spacewar"));
    CHECK(!fmt::NameIsUsable("S p a c e w a r"));
    CHECK(fmt::NameIsUsable("Persona 5 Royal"));
    // MakeAppIdBlob: magic + version + LE appid.
    const std::string blob = fmt::MakeAppIdBlob(0x01020304u);
    CHECK(blob.size() == 4 + 1 + 4);
    CHECK(blob.compare(0, 4, ac::constants::kAppIdBlobMagic) == 0);
    CHECK(static_cast<std::uint8_t>(blob[4]) == ac::constants::kAppIdBlobVersion);
    CHECK(static_cast<std::uint8_t>(blob[5]) == 0x04);
    CHECK(static_cast<std::uint8_t>(blob[6]) == 0x03);
    CHECK(static_cast<std::uint8_t>(blob[7]) == 0x02);
    CHECK(static_cast<std::uint8_t>(blob[8]) == 0x01);
    // WithAppIdSuffix (invisible): mark + 6 VS nibbles encoding the appid.
    ac::Settings st;
    st.presenceSuffixInvisible = true;
    const std::string inv = fmt::WithAppIdSuffix("Game", 0xABCDEFu, st);
    CHECK(inv.rfind("Game", 0) == 0);
    CHECK(inv.find(ac::constants::kExtraInfoInvisibleMark) == 4);
    std::uint32_t decoded = 0;
    std::size_t pos = 4 + 3;  // skip name + mark
    for (std::size_t i = 0; i < ac::constants::kExtraInfoInvisibleDigits; ++i) {
        CHECK(pos + 3 <= inv.size());
        CHECK(static_cast<std::uint8_t>(inv[pos]) == 0xEE);
        CHECK(static_cast<std::uint8_t>(inv[pos + 1]) == 0xB8);
        const std::uint32_t nib = static_cast<std::uint8_t>(inv[pos + 2]) & 0x0Fu;
        decoded = (decoded << 4) | nib;
        pos += 3;
    }
    CHECK(decoded == 0xABCDEFu);
    st.presenceSuffixInvisible = false;
    CHECK(fmt::WithAppIdSuffix("Game", 480u, st) == std::string("Game") + ac::constants::kExtraInfoAppIdSep + "480");
    // ExtractStringKVs: nested struct + string pairs + end markers.
    const std::uint8_t kv[] = {
        0x00, 'a', 0,                       // struct "a"
        0x01, 'k', '1', 0, 'v', '1', 0,     // string k1=v1
        0x08,                               // end struct
        0x01, 'k', '2', 0, 'v', '2', 0,     // string k2=v2
        0x08,                               // end root
    };
    std::vector<std::pair<std::string, std::string>> kvs;
    fmt::ExtractStringKVs(kv, sizeof(kv), kvs);
    CHECK(kvs.size() == 2);
    CHECK(kvs[0].first == "k1" && kvs[0].second == "v1");
    CHECK(kvs[1].first == "k2" && kvs[1].second == "v2");
    // Truncated input stops cleanly keeping parsed-so-far pairs.
    kvs.clear();
    fmt::ExtractStringKVs(kv, 8, kvs);   // cuts mid first value
    CHECK(kvs.empty());
}

void DonorPoolTest() {
    namespace dp = ac::hooks::DonorPool;
    CHECK(dp::kPoolCount == 15);
    CHECK(dp::DonorSteamId(14) == 76561198028121353ULL);  // LumaCore default donor
    CHECK(dp::DonorSteamId(0) != dp::DonorSteamId(1));
    // Fresh app starts at the LumaCore default index (14).
    const ac::steam::AppId app = 999001u;
    CHECK(dp::PickIndex(app) == 14);
    // A data-less answer advances the round-robin.
    dp::NoteAttemptResult(app, 14, /*okWithData=*/false);
    CHECK(dp::PickIndex(app) == 0);
    // A data-bearing answer pins the donor as preferred.
    dp::NoteAttemptResult(app, 3, /*okWithData=*/true);
    CHECK(dp::PickIndex(app) == 3);
    // Pending client-stats correlation: record then take consumes it.
    dp::StatAttempt a; a.appId = app; a.poolIndex = 3; a.seen = dp::Clock::now();
    dp::RecordPendingClientStats(app, a);
    dp::StatAttempt out;
    CHECK(dp::TakePendingClientStats(app, out));
    CHECK(out.appId == app && out.poolIndex == 3);
    CHECK(!dp::TakePendingClientStats(app, out));  // consumed
    // Service-path correlation by jobid.
    dp::StatAttempt b; b.appId = app; b.poolIndex = 7;
    dp::RecordAttempt(b, /*hasJobId=*/true, 4242ull);
    dp::StatAttempt got;
    CHECK(dp::ResolveAttempt(true, 4242ull, got) == dp::Correl::Resolved);
    CHECK(got.appId == app && got.poolIndex == 7);
    CHECK(dp::ResolveAttempt(true, 9999ull, got) == dp::Correl::NoMatch);
}


// ---------------------------------------------------------------------------
// Batch 7-9 cases
// ---------------------------------------------------------------------------

void KeyValuesTest() {
    namespace kv1 = ac::kv1;
    // Blob: root { s1 { str "hello"; i32 = 7 }  str "world"; end; end }
    const std::uint8_t blob[] = {
        0x00, 's','1', 0,                       // dict "s1"
        0x01, 'k','e','y', 0, 'h','e','l','l','o', 0,   // string
        0x02, 'n', 0, 7, 0, 0, 0,               // int32
        0x08,                                   // end s1
        0x01, 't','o','p', 0, 'w','o','r','l','d', 0,   // string
        0x08,                                   // end root
    };
    // Visitor: depth + callback order.
    struct Rec final : kv1::Visitor {
        std::vector<std::string> events;
        void OnDictBegin(const std::string& n, int d) override { events.push_back("D+" + std::to_string(d) + ":" + n); }
        void OnDictEnd(int d) override { events.push_back("D-" + std::to_string(d)); }
        void OnString(const std::string& k, const std::string& v, int d) override { events.push_back("S" + std::to_string(d) + ":" + k + "=" + v); }
        void OnInt32(const std::string& k, std::uint8_t, std::int32_t v, int d) override { events.push_back("I" + std::to_string(d) + ":" + k + "=" + std::to_string(v)); }
    } rec;
    CHECK(kv1::WalkBinary(blob, sizeof(blob), rec));
    CHECK(rec.events.size() == 5);
    CHECK(rec.events[0] == "D+0:s1");
    CHECK(rec.events[1] == "S1:key=hello");
    CHECK(rec.events[2] == "I1:n=7");
    CHECK(rec.events[3] == "D-0");
    CHECK(rec.events[4] == "S0:top=world");
    // ReadStringKVs: flat pairs at any depth.
    auto pairs = kv1::ReadStringKVs(blob, sizeof(blob));
    CHECK(pairs.size() == 2);
    CHECK(pairs[0].first == "key" && pairs[0].second == "hello");
    CHECK(pairs[1].first == "top" && pairs[1].second == "world");
    // Writer round-trip.
    const std::string written = kv1::WriteStringKVs({{"a", "1"}, {"b", "two"}});
    auto back = kv1::ReadStringKVs(reinterpret_cast<const std::uint8_t*>(written.data()), written.size());
    CHECK(back.size() == 2 && back[0].first == "a" && back[0].second == "1" &&
          back[1].first == "b" && back[1].second == "two");
    // Malformed: unknown type -> false.
    const std::uint8_t bad[] = {0x77, 'x', 0};
    struct Empty final : kv1::Visitor {} empty;
    CHECK(!kv1::WalkBinary(bad, sizeof(bad), empty));
    // Truncated string -> false.
    const std::uint8_t trunc[] = {0x01, 'k', 0, 'v'};
    CHECK(!kv1::WalkBinary(trunc, sizeof(trunc), empty));
}

void VdfTextTest() {
    namespace vdf = ac::vdf;
    std::string k, v;
    CHECK(vdf::ParseLine("\t\"apps\" \t ", k, v) == vdf::LineKind::SectionKey && k == "apps");
    CHECK(vdf::ParseLine("\"LastPlayed\" \t \"1700000000\"", k, v) == vdf::LineKind::KeyValue &&
          k == "LastPlayed" && v == "1700000000");
    CHECK(vdf::ParseLine("{", k, v) == vdf::LineKind::None);
    CHECK(vdf::ParseLine("\"key\" {", k, v) == vdf::LineKind::None);
    CHECK(vdf::ParseLine("garbage", k, v) == vdf::LineKind::None);
    // Unterminated quote.
    CHECK(vdf::ParseLine("\"open", k, v) == vdf::LineKind::None);
    std::size_t pos = 0; char brace = 0;
    CHECK(vdf::FindUnquotedBrace("\"quoted { brace\"", pos, brace) == false);
    CHECK(vdf::FindUnquotedBrace("\"a\" \"b\" }", pos, brace) && brace == '}');
    CHECK(vdf::FindUnquotedBrace("x { y", pos, brace) && brace == '{' && pos == 2);
    std::string esc = "C:\\\\Games\\\\Steam";   // VDF-escaped C:\Games\Steam
    vdf::UnescapeBackslashes(esc);
    CHECK(esc == "C:\\Games\\Steam");
    const std::string content =
        "\"libraryfolders\"\n{\n"
        "  \"0\"\n  {\n"
        "    \"path\"  \"C:\\\\Steam\"\n"
        "    \"PATH\"  \"D:\\\\Games\"\n"
        "  }\n}\n";
    const auto paths = vdf::ExtractQuotedValues(content, "path");   // case-insensitive
    CHECK(paths.size() == 2);
    CHECK(paths[0] == "C:\\Steam");
    CHECK(paths[1] == "D:\\Games");
}

void JsonEscapesTest() {
    namespace ju = ac::jsonutil;
    std::string out;
    CHECK(ju::PullEscapedStringField("{\"key\": \"value\"}", "key", out) && out == "value");
    CHECK(ju::PullEscapedStringField("{\"key\": \"a\\\"b\\\\c\\/d\\n\\t\\r\\b\\f\"}", "key", out) &&
          out == "a\"b\\c/d\n\t\r\b\f");
    CHECK(!ju::PullEscapedStringField("{\"key\": \"bad\\u0041\"}", "key", out));   // \u rejected
    CHECK(!ju::PullEscapedStringField("{\"key\": \"bad\\q\"}", "key", out));        // unknown rejected
    CHECK(ju::PullEscapedStringField("{\"key\": \"\"}", "key", out) && out.empty());  // empty ok
    CHECK(!ju::PullEscapedStringField("{\"other\": \"x\"}", "key", out));             // missing
    CHECK(!ju::PullEscapedStringField("{\"key\": \"unterminated", "key", out));
    CHECK(!ju::PullEscapedStringField("{\"key\": 42}", "key", out));                   // not a string
    // Whitespace around colon is tolerated.
    CHECK(ju::PullEscapedStringField("{\"key\" :\n\"v\"}", "key", out) && out == "v");
}

void ManifestIdentityTest() {
    namespace mi = ac::manifestidentity;
    auto putU32 = [](std::string& s, std::uint32_t v) {
        s.push_back(static_cast<char>(v & 0xFF));
        s.push_back(static_cast<char>((v >> 8) & 0xFF));
        s.push_back(static_cast<char>((v >> 16) & 0xFF));
        s.push_back(static_cast<char>((v >> 24) & 0xFF));
    };
    auto putVarint = [](std::string& s, std::uint64_t v) {
        while (v >= 0x80) { s.push_back(static_cast<char>((v & 0x7F) | 0x80)); v >>= 7; }
        s.push_back(static_cast<char>(v));
    };
    const std::uint32_t depot = 12345u;
    const std::uint64_t gid = 7719911223344556677ULL;
    std::string meta;
    meta.push_back(0x08); putVarint(meta, depot);        // field 1, varint
    meta.push_back(0x10); putVarint(meta, gid);          // field 2, varint
    meta.push_back(0x1A); putVarint(meta, 3); meta += "abc";   // field 3, length-delimited skip
    meta.push_back(0x09); meta.append(8, '\0');         // field 1, fixed64 skip
    meta.push_back(0x0D); meta.append(4, '\0');         // field 1, fixed32 skip
    std::string blob;
    putU32(blob, 0x71F617D0u); putU32(blob, 2); blob += "XX";   // header w/ payload
    putU32(blob, 0x1F4812BEu); putU32(blob, static_cast<std::uint32_t>(meta.size())); blob += meta;
    CHECK(mi::ManifestIdentityMatches(blob, depot, gid));
    CHECK(!mi::ManifestIdentityMatches(blob, depot + 1, gid));
    CHECK(!mi::ManifestIdentityMatches(blob, depot, gid + 1));
    CHECK(!mi::ManifestIdentityMatches(std::string_view(blob).substr(0, blob.size() - 1), depot, gid));
    std::string badMagic = blob; putU32(badMagic = badMagic.substr(0, 4), 0x12345678u);
    CHECK(!mi::ManifestIdentityMatches(badMagic, depot, gid));
    // Unknown wire type is rejected.
    std::string weirdMeta;
    weirdMeta.push_back(0x08); putVarint(weirdMeta, depot);
    weirdMeta.push_back(0x0B);                            // wire type 3: unsupported
    std::string weirdBlob;
    putU32(weirdBlob, 0x71F617D0u); putU32(weirdBlob, 0);
    putU32(weirdBlob, 0x1F4812BEu); putU32(weirdBlob, static_cast<std::uint32_t>(weirdMeta.size()));
    weirdBlob += weirdMeta;
    CHECK(!mi::ManifestIdentityMatches(weirdBlob, depot, gid));
    // Unit checks for the readers themselves.
    std::size_t off = 0; std::uint32_t u = 0;
    CHECK(mi::ReadU32Le(std::string_view("\x01\x00\x00\x80", 4), off, u) && u == 0x80000001u && off == 4);
    CHECK(!mi::ReadU32Le(std::string_view("\x01\x00", 2), off, u));
    off = 0; std::uint64_t w = 0;
    CHECK(mi::ReadVarint(std::string_view("\xAC\x02", 2), off, w) && w == 300 && off == 2);
    CHECK(!mi::ReadVarint(std::string_view("\x80", 1), off, w));   // truncated varint
}

void IpcReplyTest() {
    namespace ir = ac::hooks::ipcreply;
    std::uint8_t storage[64] = {};
    ac::steam::CUtlBuffer buf{};
    buf.memory.memory = storage;
    buf.put = 64;
    CHECK(ir::CanWrite(&buf, 64));
    CHECK(!ir::CanWrite(&buf, 65));
    CHECK(ir::Begin(&buf));
    CHECK(storage[0] == ac::constants::kIpcReplyTag);
    CHECK(ir::WriteU32(&buf, 4, 0xDEADBEEFu));
    CHECK(storage[4] == 0xEF && storage[5] == 0xBE && storage[6] == 0xAD && storage[7] == 0xDE);
    CHECK(ir::WriteU64(&buf, 8, 0x1122334455667788ULL));
    CHECK(storage[8] == 0x88 && storage[15] == 0x11);
    CHECK(ir::WriteAt(&buf, 60, storage + 4, 4));           // exact fit
    CHECK(!ir::WriteAt(&buf, 61, storage + 4, 4));          // overflow refused
    CHECK(!ir::WriteU32(&buf, 62, 1u));                     // overflow refused
    CHECK(ir::WriteAt(&buf, 10, storage, 0));               // zero-byte write ok
    // Tiny buffer: nothing may be written.
    // NB: non chiamarla "small": l'SDK Windows (rpcndr.h) definisce
    // #define small char e MSVC espanderebbe il nome della variabile.
    std::uint8_t tiny[1] = {0};
    ac::steam::CUtlBuffer tinyBuf{}; tinyBuf.memory.memory = tiny; tinyBuf.put = 0;
    CHECK(!ir::CanWrite(&tinyBuf, 1));
    CHECK(!ir::Begin(&tinyBuf));
    CHECK(!ir::WriteU32(&tinyBuf, 0, 7u));
    // Null safety.
    CHECK(!ir::CanWrite(nullptr, 1));
    CHECK(!ir::Begin(nullptr));
    CHECK(!ir::WriteAt(&buf, 0, nullptr, 4));
}

void SnapshotMergeTest() {
    namespace snap = ac::backup::snapshot;
    snap::SnapshotData d;
    snap::MergeUnlock(d, 10, 0);                            // baseline, unknown time
    CHECK(snap::HasUnlock(d, 10) && d.unlocks[0].unlockTime == 0);
    snap::MergeUnlock(d, 10, 1700000000u);                  // real time replaces 0
    CHECK(d.unlocks[0].unlockTime == 1700000000u);
    snap::MergeUnlock(d, 10, 1800000000u);                  // later time must NOT win
    CHECK(d.unlocks[0].unlockTime == 1700000000u);
    snap::MergeUnlock(d, 10, 1600000000u);                  // earlier time wins
    CHECK(d.unlocks[0].unlockTime == 1600000000u);
    snap::MergeUnlock(d, 10, 0);                            // 0 never overwrites real time
    CHECK(d.unlocks[0].unlockTime == 1600000000u);
    snap::MergeUnlock(d, 11, 1500000000u);
    CHECK(d.unlocks.size() == 2);
    snap::MergeStat(d, 5, 42u);                             // last committed value wins
    CHECK(snap::HasStat(d, 5) && d.stats[0].value == 42u);
    snap::MergeStat(d, 5, 43u);
    CHECK(d.stats[0].value == 43u);
    snap::MergeStat(d, 6, 1u);
    snap::MergeUnlock(d, 3, 1u);
    snap::SortAll(d);
    CHECK(d.unlocks[0].id == 3 && d.unlocks[1].id == 10 && d.unlocks[2].id == 11);
    CHECK(d.stats[0].id == 5 && d.stats[1].id == 6);
}


// --- QW1: TtlCache (cattura anche la regressione del Get sotto lock) ---
void TtlCacheTest() {
    using ac::utils::TtlCache;
    {
        TtlCache<std::string, int> c(2, std::chrono::seconds(60));
        c.Put("a", 1);
        CHECK(c.Get("a") == 1);
        CHECK(!c.Get("missing").has_value());
        c.Put("b", 2);
        c.Put("c", 3);                       // sfora il cap di 2: LRU ("a") evicta
        CHECK(c.Size() == 2);
        CHECK(!c.Get("a").has_value());      // la più vecchia è uscita
        CHECK(c.Get("b") == 2);
        CHECK(c.Get("c") == 3);
    }
    {
        TtlCache<std::string, int> c(8, std::chrono::seconds(1));
        c.Put("short", 42);
        CHECK(c.Get("short") == 42);
        std::this_thread::sleep_for(std::chrono::milliseconds(1050));
        CHECK(!c.Get("short").has_value());  // scaduta
        CHECK(c.EvictionCount() >= 1);       // la scadenza conta come eviction
        c.Put("fresh", 7);
        CHECK(c.Get("fresh") == 7);          // una entry nuova torna valida
    }
    {
        TtlCache<std::string, int> c(8, std::chrono::seconds(60));
        c.PutNegative("nope");
        CHECK(c.NegativeCount() == 1);
        CHECK(c.Get("nope") == 0);           // negative hit: valore default
    }
}

// --- QW8: HexCodec (roundtrip + reject deterministici) ---
void HexCodecTest() {
    using namespace ac::hex;
    const std::vector<std::uint8_t> blob = {0x48, 0x8B, 0x05, 0x00, 0xFF};
    const std::string hex = Encode(blob);
    CHECK(hex == "488B0500FF");              // Encode è uppercase
    CHECK(Decode(hex) == blob);
    CHECK(Decode("488b05") == std::vector<std::uint8_t>({0x48, 0x8B, 0x05})); // lowercase ok
    const auto empty = Decode("");
    CHECK(empty.has_value() && empty->empty());
    CHECK(!Decode("abc").has_value());       // lunghezza dispari
    CHECK(!Decode("zz").has_value());        // carattere non hex
    CHECK(!Decode(" 48").has_value());       // lo spazio non è hex
    CHECK(!Decode("0x48").has_value());      // niente prefissi
}

// --- QW8: SignatureCodec (parser signature estratto da PatternEngine) ---
void SignatureTest() {
    using ac::pattern::ParseSignature;
    std::vector<std::uint8_t> bytes;
    std::string mask;
    CHECK(ParseSignature("48 8B ?? C3", bytes, mask));
    CHECK(bytes.size() == 4 && mask.size() == 4);
    CHECK(mask == "xx?x");
    CHECK(bytes[0] == 0x48 && bytes[1] == 0x8B && bytes[3] == 0xC3);
    CHECK(ParseSignature("00", bytes, mask));          // 0x00 è un byte valido
    CHECK(bytes.size() == 1 && mask == "x" && bytes[0] == 0x00);
    CHECK(ParseSignature("a1", bytes, mask));          // lowercase accettato
    CHECK(bytes.size() == 1 && bytes[0] == 0xA1);
    CHECK(!ParseSignature("", bytes, mask));           // vuoto
    CHECK(!ParseSignature("???", bytes, mask));        // wildcard malformata
    CHECK(!ParseSignature("4Z", bytes, mask));         // cifra non hex
    CHECK(!ParseSignature("1FF", bytes, mask));        // token troppo lungo
}

// --- QW8: IpcSpecParse (parser TOML puro estratto da IpcSpec) ---
void IpcSpecParseTest() {
    using ac::ipcspec::MethodSpec;
    using ac::ipcspec::ParseSpecToml;
    std::unordered_map<std::string, std::uint8_t> ifaces;
    std::unordered_map<std::string, MethodSpec> methods;

    const std::string good =
        "[IClientUser]\n"
        "interface_id = 5\n"
        "[IClientUser.GetSteamID]\n"
        "funcHash = \"0x1A2B3C4D\"\n"
        "fencepost = \"1C\"\n"
        "argc = 3\n";
    CHECK(ParseSpecToml(good, ifaces, methods));
    CHECK(ifaces.size() == 1 && ifaces.at("IClientUser") == 5);
    CHECK(methods.size() == 1);
    const auto it = methods.find("IClientUser::GetSteamID");
    CHECK(it != methods.end());
    CHECK(it->second.hash == 0x1A2B3C4Du);
    CHECK(it->second.fencepost == 0x1Cu);
    CHECK(it->second.argc == 3u);

    // interface_id fuori range: l'intera interfaccia (metodi inclusi) è scartata.
    ifaces.clear(); methods.clear();
    CHECK(!ParseSpecToml(
        "[IClientFriends]\ninterface_id = 999\n"
        "[IClientFriends.GetPersonaName]\nfuncHash = \"ABCD\"\n",
        ifaces, methods));

    // funcHash malformato: metodo scartato; nessun metodo valido → parse fallito.
    ifaces.clear(); methods.clear();
    CHECK(!ParseSpecToml(
        "[IClientUser]\n[IClientUser.Broken]\nfuncHash = \"nothex\"\n",
        ifaces, methods));

    // Documento vuoto o TOML invalido → false (lo stato esistente non va toccato).
    ifaces.clear(); methods.clear();
    CHECK(!ParseSpecToml("", ifaces, methods));
    CHECK(!ParseSpecToml("[broken\nkey = 1", ifaces, methods));

    // funcHash senza prefisso 0x e valore minimo accettato (1).
    ifaces.clear(); methods.clear();
    CHECK(ParseSpecToml("[I]\n[I.M]\nfuncHash = \"1\"\n", ifaces, methods));
    CHECK(methods.at("I::M").hash == 1u);

    // funcHash zero rifiutato (hash 0 = metodo assente, per contratto).
    ifaces.clear(); methods.clear();
    CHECK(!ParseSpecToml("[I]\n[I.M]\nfuncHash = \"0\"\n", ifaces, methods));

    // argc negativo ignorato, il metodo resta valido.
    ifaces.clear(); methods.clear();
    CHECK(ParseSpecToml("[I]\n[I.M2]\nfuncHash = \"AB\"\nargc = -1\n", ifaces, methods));
    CHECK(methods.at("I::M2").argc == 0u);
}


// --- I-C: JsonWriter (golden test byte-per-byte del formato schema 9) ---
void JsonWriterTest() {
    using ac::jsonw::Writer;
    // Copertura escape carattere-per-carattere.
    CHECK(ac::jsonw::Escape(std::string("\"\\\n\r\t\x01") + "A") ==
          "\\\"\\\\\\n\\r\\t\\u0001A");
    CHECK(ac::jsonw::Escape("plain ascii 123") == "plain ascii 123");

    Writer w;
    w.Int64("schema_version", 9);
    w.Int64("ts", 1727950000LL);
    w.Str("build_id", std::string("b\"q\\s\nx") + '\x01' + "z");
    w.Bool("flag_t", true);
    w.Bool("flag_f", false);
    w.StrArray("empty_list", {});
    w.StrArray("names", {"alpha", "b\"eta"});
    w.ObjectArray("empty_objs");
    w.ArrayEnd();
    w.ObjectArray("diagnostics");
    w.RawObject("{\"ts_ms\": 1, \"category\": \"" + ac::jsonw::Escape("wire_eresult") +
                "\", \"detail\": \"" + ac::jsonw::Escape("a=b") + "\"}");
    w.RawObject("{\"ts_ms\": 2, \"category\": \"" + ac::jsonw::Escape("x") +
                "\", \"detail\": \"" + ac::jsonw::Escape("y") + "\"}");
    w.ArrayEnd();
    const std::string out = w.Finish();

    // Golden: ESATTAMENTE il formato pubblicato dallo schema_version 9
    // (indent 2 spazi, array su righe proprie, ultimo campo senza virgola).
    const std::string golden =
        "{\n"
        "  \"schema_version\": 9,\n"
        "  \"ts\": 1727950000,\n"
        "  \"build_id\": \"b\\\"q\\\\s\\nx\\u0001z\",\n"
        "  \"flag_t\": true,\n"
        "  \"flag_f\": false,\n"
        "  \"empty_list\": [],\n"
        "  \"names\": [\n"
        "    \"alpha\",\n"
        "    \"b\\\"eta\"\n"
        "  ],\n"
        "  \"empty_objs\": [],\n"
        "  \"diagnostics\": [\n"
        "    {\"ts_ms\": 1, \"category\": \"wire_eresult\", \"detail\": \"a=b\"},\n"
        "    {\"ts_ms\": 2, \"category\": \"x\", \"detail\": \"y\"}\n"
        "  ]\n"
        "}\n";
    CHECK(out == golden);
}


// --- I-E: contract fixture DLL<->Desk per l'identità manifest (D4) ---
// Gli STESSI blob in Tools/contract_fixtures/manifest sono asseriti qui e in
// AetherDesk/src-tauri/src/tests/manifest_contract_tests.rs: se un lato
// cambia verdetto, uno dei due test si rompe.
void ManifestContractTest() {
    namespace mi = ac::manifestidentity;
    constexpr std::uint32_t kDepot = 489831;
    constexpr std::uint64_t kGid = 4940892828028256588ull;
    const fs::path dir = CONTRACT_FIXTURES_DIR;

    const auto Read = [&](const char* name) {
        std::ifstream f(dir / name, std::ios::binary);
        CHECK(f.is_open());
        std::string body((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
        return body;
    };

    // Validi: identità letta dal metadata, match solo con gli attesi esatti.
    const std::string valid = Read("valid_basic.bin");
    CHECK(mi::ManifestIdentityMatches(valid, kDepot, kGid));
    CHECK(!mi::ManifestIdentityMatches(valid, kDepot + 1, kGid));
    CHECK(!mi::ManifestIdentityMatches(valid, kDepot, kGid + 1));

    const std::string extra = Read("valid_extra_wiretypes.bin");
    CHECK(mi::ManifestIdentityMatches(extra, kDepot, kGid));

    // Malformati: tutti rifiutati (anche con attesi zero).
    for (const char* bad : {"bad_magic_payload.bin", "bad_magic_metadata.bin",
                            "truncated_payload.bin", "truncated_metadata_varint.bin",
                            "bad_wire_type.bin", "empty.bin", "garbage_short.bin"}) {
        const std::string body = Read(bad);
        CHECK(!mi::ManifestIdentityMatches(body, kDepot, kGid));
        CHECK(!mi::ManifestIdentityMatches(body, 0, 0));
    }
}

int main(int argc, char** argv) {
    try {
        CHECK(argc == 2);
        const std::string test = argv[1];
        if (test == "strings") Strings(); else if (test == "paths") Paths();
        else if (test == "policy") Policy(); else if (test == "worker") Worker();
        else if (test == "budget") Budget(); else if (test == "settings") Settings();
        else if (test == "registry") Registry();
        else if (test == "workers") Workers();
        else if (test == "schema_buckets") SchemaBuckets();
        else if (test == "workers_race") WorkersShutdownRace();
        else if (test == "netpacket") NetPacket();
        else if (test == "sentinel") Sentinel();
        else if (test == "structguard") StructGuard();
        else if (test == "gamesplayed_format") GamesPlayedFormatTest();
        else if (test == "donorpool") DonorPoolTest();
        else if (test == "keyvalues") KeyValuesTest();
        else if (test == "vdftext") VdfTextTest();
        else if (test == "jsonutil_escapes") JsonEscapesTest();
        else if (test == "manifest_identity") ManifestIdentityTest();
        else if (test == "ipcreply") IpcReplyTest();
        else if (test == "snapshot_merge") SnapshotMergeTest();
        else if (test == "ttlcache") TtlCacheTest();
        else if (test == "hexcodec") HexCodecTest();
        else if (test == "signature") SignatureTest();
        else if (test == "ipcspec") IpcSpecParseTest();
        else if (test == "jsonwriter") JsonWriterTest();
        else if (test == "manifest_contract") ManifestContractTest(); else CHECK(false);
        std::cout << test << ": PASS\n";
        return 0;
    } catch (const std::exception& e) { std::cerr << e.what() << '\n'; return 1; }
}
