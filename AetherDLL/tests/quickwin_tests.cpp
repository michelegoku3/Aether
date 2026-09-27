#include "pch.h"
#include "core/Settings.h"
#include "core/HookManager.h"
#include "core/NetPacketAbi.h"
#include "utils/Strings.h"
#include "utils/DeskPaths.h"
#include "utils/CoalescingWorker.h"
#include "utils/LogBurstBudget.h"
#include "propagate/PropagationPolicy.h"
#include "core/Workers.h"
#include "MinHook.h"
#include <atomic>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <stdexcept>
#include <thread>
#include <cstring>
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
    w::Shutdown();
    CHECK(sawStop.load());              // lo stop flag è arrivato
    w::Shutdown();                      // idempotente
    CHECK(!w::Submit([&] { counter.fetch_add(1); }));   // rifiuto post-shutdown
    CHECK(!w::StartWorker("late", [](std::atomic<bool>&) {}));
    CHECK(counter.load() == 101);       // nessun job fantasma
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

int main(int argc, char** argv) {
    try {
        CHECK(argc == 2);
        const std::string test = argv[1];
        if (test == "strings") Strings(); else if (test == "paths") Paths();
        else if (test == "policy") Policy(); else if (test == "worker") Worker();
        else if (test == "budget") Budget(); else if (test == "settings") Settings();
        else if (test == "registry") Registry();
        else if (test == "workers") Workers();
        else if (test == "netpacket") NetPacket(); else CHECK(false);
        std::cout << test << ": PASS\n";
        return 0;
    } catch (const std::exception& e) { std::cerr << e.what() << '\n'; return 1; }
}
