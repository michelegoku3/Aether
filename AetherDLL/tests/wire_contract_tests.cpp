#include "pch.h"

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "core/Settings.h"
#include "hooks/wire/BackupIo.h"
#include "hooks/wire/ManifestRestore.h"
#include "network/HubcapQuota.h"
#include "network/ManifestFetch.h"
#include "network/RuntimeHttp.h"
#include "utils/RemoteInject.h"
#include "utils/WinFile.h"

#include <atomic>
#include <chrono>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <set>
#include <stdexcept>
#include <string>
#include <thread>
#include <utility>
#include <vector>

#define CHECK(expr)                                                                                    \
    do {                                                                                               \
        if (!(expr))                                                                                   \
            throw std::runtime_error("Check failed at line " + std::to_string(__LINE__) + ": " #expr); \
    } while (false)

namespace fs = std::filesystem;
using namespace std::chrono_literals;
namespace ManifestRestore = ac::hooks::ManifestRestore;

// Isolated Steam root + AetherData per fixture. Removed on destruction.
struct Fixture {
    fs::path dir = fs::temp_directory_path() /
                   ("aether-wire-" + std::to_string(std::chrono::steady_clock::now().time_since_epoch().count()));
    fs::path steam;
    fs::path luaDir;
    fs::path depotcache;
    fs::path deskData;

    Fixture() {
        static std::atomic<unsigned> seq{0};
        dir += "-" + std::to_string(seq.fetch_add(1));
        steam = dir / "Steam";
        luaDir = steam / "plug-in";
        depotcache = steam / "depotcache";
        deskData = dir / "AetherData";
        fs::create_directories(luaDir);
        fs::create_directories(depotcache);
        fs::create_directories(deskData);
        ac::g_state.steamInstallPath = steam.string();
        ac::g_state.luaDir = luaDir.string();
        ac::g_state.deskDataDir = deskData.string();
        const fs::path toml = dir / "aethercore.toml";
        {
            std::ofstream out(toml, std::ios::binary);
            out << "[manifest_cache]\nrestore_on_startup=true\n";
        }
        CHECK(ac::Settings::Initialize(toml.string()));
    }
    ~Fixture() {
        std::error_code ec;
        fs::remove_all(dir, ec);
    }

    void Write(const fs::path& path, const std::string& data) {
        std::ofstream out(path, std::ios::binary | std::ios::trunc);
        out.write(data.data(), static_cast<std::streamsize>(data.size()));
        CHECK(out.good());
    }
    std::string Read(const fs::path& path) {
        std::ifstream in(path, std::ios::binary);
        CHECK(in.is_open());
        return std::string((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
    }
    // Any staging residue left behind by a copy (should always be empty).
    std::vector<fs::path> StagingResidue() {
        std::vector<fs::path> out;
        std::error_code ec;
        for (auto it = fs::recursive_directory_iterator(deskData, ec); !ec && it != fs::recursive_directory_iterator();
             it.increment(ec)) {
            if (it->is_regular_file() && it->path().string().find(".aether-tmp") != std::string::npos)
                out.push_back(it->path());
        }
        return out;
    }
};

// W2 mechanism: staging names are unique per call and stay in the target dir.
void StagingUnique() {
    const std::string dest = "C:\\fakeroot\\backup\\100_200.manifest";
    std::set<std::string> seen;
    for (int i = 0; i < 2000; ++i) {
        const std::string tmp = ac::backup::io::UniqueStagingPath(dest);
        CHECK(seen.insert(tmp).second);                                     // never repeats, even back-to-back
        CHECK(fs::path(tmp).parent_path() == fs::path(dest).parent_path()); // same volume
        CHECK(tmp.find(".aether-tmp-") != std::string::npos);
    }
}

// W1: size verification names the failing side instead of masking it.
void SizeVerify() {
    Fixture f;
    const fs::path present = f.dir / "present.bin";
    const fs::path missing = f.dir / "missing.bin";
    f.Write(present, "0123456789");

    const auto bothMissing = ac::backup::io::VerifySameSize(missing.string(), missing.string());
    CHECK(!bothMissing.ok);
    CHECK(!bothMissing.srcError.empty() && !bothMissing.dstError.empty());

    const auto same = ac::backup::io::VerifySameSize(present.string(), present.string());
    CHECK(same.ok);
    CHECK(same.srcError.empty() && same.dstError.empty());

    const auto dstMissing = ac::backup::io::VerifySameSize(present.string(), missing.string());
    CHECK(!dstMissing.ok);
    CHECK(dstMissing.srcError.empty() && !dstMissing.dstError.empty());

    const auto srcMissing = ac::backup::io::VerifySameSize(missing.string(), present.string());
    CHECK(!srcMissing.ok);
    CHECK(!srcMissing.srcError.empty() && srcMissing.dstError.empty());
}

// Shared copy contract rules 2/3/4 (+W8 commented pins) through the public
// post-generation entry point.
void BackupRules() {
    Fixture f;
    const std::string payload = "MANIFEST-PAYLOAD-100-200";
    f.Write(f.depotcache / "100_200.manifest", payload);
    f.Write(f.depotcache / "999_888.manifest", "UNREFERENCED");
    // Active pin + commented pin (contract: both count as references).
    f.Write(f.luaDir / "480.lua", "setManifestid(100,\"200\")\n");
    f.Write(f.luaDir / "481.lua", "--setManifestid(100,\"200\")\n");

    ManifestRestore::BackupManifestAfterGeneration(100, 200);
    const fs::path dst480 = f.deskData / "backup" / "480" / "lua" / "100_200.manifest";
    const fs::path dst481 = f.deskData / "backup" / "481" / "lua" / "100_200.manifest";
    CHECK(f.Read(dst480) == payload); // rule 4: copied and byte-identical
    CHECK(f.Read(dst481) == payload); // commented pin counts as a reference
    CHECK(!fs::exists(f.deskData / "backup" / "480" / "lua" / "999_888.manifest")); // unreferenced: never copied
    CHECK(f.StagingResidue().empty());

    // Rule 2: a non-empty destination is never overwritten, even by different bytes.
    f.Write(dst480, "DIFFERENT-NON-EMPTY");
    ManifestRestore::BackupManifestAfterGeneration(100, 200);
    CHECK(f.Read(dst480) == "DIFFERENT-NON-EMPTY");

    // Rule 3: a 0-byte placeholder is replaced with the real bytes.
    f.Write(dst481, "");
    CHECK(fs::file_size(dst481) == 0);
    ManifestRestore::BackupManifestAfterGeneration(100, 200);
    CHECK(f.Read(dst481) == payload);
    CHECK(f.StagingResidue().empty());
}

// B1: a present-but-unreadable quota file refuses generation (fail closed)
// and preserves a .corrupt copy; a missing file starts fresh (allowed).
void QuotaCorrupt() {
    Fixture f;
    const fs::path stateDir = f.deskData / "state";
    std::error_code ec;
    fs::create_directories(stateDir, ec);
    const fs::path quota = stateDir / "hubcap_generation_quota.json";

    f.Write(quota, "not-json{{{");
    CHECK(!ac::hubcapquota::TryReserveGameGeneration());
    bool corruptKept = false;
    for (auto it = fs::directory_iterator(stateDir, ec); !ec && it != fs::directory_iterator(); it.increment(ec)) {
        if (it->path().string().find(".corrupt-") != std::string::npos)
            corruptKept = true;
    }
    CHECK(corruptKept);

    fs::remove(quota, ec);
    for (auto it = fs::directory_iterator(stateDir, ec); !ec && it != fs::directory_iterator(); it.increment(ec)) {
        if (it->path().string().find(".corrupt-") != std::string::npos)
            fs::remove(it->path(), ec);
    }
    CHECK(ac::hubcapquota::TryReserveGameGeneration());
    ac::hubcapquota::ReleaseGameGeneration();
}

// F4: the inverted backup index serves lookups without per-miss walks.
// A manifest present only in backup/<app>/lua is found, published into
// depotcache, and served from the memo afterwards.
void ManifestIndexBackupHit() {
    Fixture f;
    const std::string payload = "INDEXED-MANIFEST";
    const fs::path appLua = f.deskData / "backup" / "9" / "lua";
    std::error_code ec;
    fs::create_directories(appLua, ec);
    f.Write(appLua / "100_200.manifest", payload);

    CHECK(ac::manifestfetch::HasLocalManifest(200, 100));
    CHECK(f.Read(f.depotcache / "100_200.manifest") == payload);
    CHECK(ac::manifestfetch::HasLocalManifest(200, 100));                // memo positive, re-verified
    CHECK(ac::manifestfetch::HasLocalManifest(999999, 888888) == false); // negative, no walk storm
}

// B5: restore publishes byte-identical copies and replaces 0-byte
// placeholders; a non-empty destination is never touched (present hit).
// The torn-write mismatch branch itself is covered by SizeVerify (same
// VerifySameSize helper); a torn copy cannot be staged through this API.
void RestoreRoundtrip() {
    Fixture f;
    const std::string payload = "MANIFEST-PAYLOAD-RESTORE";
    // RestoreLuaDirectory(luaDir, depotcache): luaDir holds backup manifests.
    const fs::path luaBackup = f.dir / "luabackup";
    std::error_code ec;
    fs::create_directories(luaBackup, ec);
    f.Write(luaBackup / "100_200.manifest", payload);
    f.Write(luaBackup / "101_201.manifest", payload);

    // Missing destination -> restored byte-identical.
    auto stats = ManifestRestore::RestoreLuaDirectory(luaBackup, f.depotcache);
    CHECK(stats.restored == 2);
    CHECK(stats.errors == 0);
    CHECK(f.Read(f.depotcache / "100_200.manifest") == payload);

    // Non-empty destination -> present hit, never overwritten.
    f.Write(f.depotcache / "100_200.manifest", "DIFFERENT");
    stats = ManifestRestore::RestoreLuaDirectory(luaBackup, f.depotcache);
    CHECK(f.Read(f.depotcache / "100_200.manifest") == "DIFFERENT");

    // 0-byte placeholder -> replaced with the real bytes.
    f.Write(f.depotcache / "101_201.manifest", "");
    stats = ManifestRestore::RestoreLuaDirectory(luaBackup, f.depotcache);
    CHECK(f.Read(f.depotcache / "101_201.manifest") == payload);
    CHECK(stats.errors == 0);
}

// W2 integration: two threads backing up the same manifest must not collide
// on the staging file and must leave no residue.
void BackupConcurrentWriters() {
    Fixture f;
    const std::string payload = "CONCURRENT-MANIFEST";
    f.Write(f.depotcache / "101_201.manifest", payload);
    f.Write(f.luaDir / "482.lua", "setManifestid(101,\"201\")\n");
    const fs::path dst = f.deskData / "backup" / "482" / "lua" / "101_201.manifest";

    std::thread a([] { ManifestRestore::BackupManifestAfterGeneration(101, 201); });
    std::thread b([] { ManifestRestore::BackupManifestAfterGeneration(101, 201); });
    a.join();
    b.join();
    CHECK(f.Read(dst) == payload);
    CHECK(f.StagingResidue().empty());
}

// W8: pin scan is case-insensitive and digit-strict through the same entry.
void PinScan() {
    Fixture f;
    f.Write(f.depotcache / "102_202.manifest", "PINNED");
    f.Write(f.depotcache / "103_203.manifest", "NOT-PINNED");
    f.Write(f.luaDir / "483.lua", "SETMANIFESTID(102,\"202\")\n");                   // icase: counts
    f.Write(f.luaDir / "484.lua", "setManifestid(abc,\"x\")\nsetManifestid(103)\n"); // invalid: ignored

    ManifestRestore::BackupManifestAfterGeneration(102, 202);
    CHECK(f.Read(f.deskData / "backup" / "483" / "lua" / "102_202.manifest") == "PINNED");
    ManifestRestore::BackupManifestAfterGeneration(103, 203);
    CHECK(!fs::exists(f.deskData / "backup" / "484" / "lua" / "103_203.manifest"));
}

// W9: Lua-facing GET/POST allow HTTPS only. Both rejections happen before any
// network I/O, so this test needs no network. The allowed-HTTPS success path
// would perform real I/O and is deliberately not covered here.
void HttpGate() {
    const auto getHttp = ac::http::Get("http://example.com/x");
    CHECK(!getHttp.networkError && getHttp.status == 403 && getHttp.body.empty());

    const auto getUpper = ac::http::Get("HTTP://example.com/");
    CHECK(!getUpper.networkError && getUpper.status == 403 && getUpper.body.empty());

    const auto postHttp = ac::http::Post("http://example.com/x", "{}", {});
    CHECK(!postHttp.networkError && postHttp.status == 403 && postHttp.body.empty());

    // Disallowed host over HTTPS is still refused without touching the network.
    const auto getBlocked = ac::http::Get("https://nonexistent.invalid/");
    CHECK(!getBlocked.networkError && getBlocked.status == 403 && getBlocked.body.empty());
}

// W7: the single publish path emits identical bytes, truncates with a marker,
// filters below the level gate, and deduplicates WriteOnce.
void LoggerGolden() {
    Fixture f;
    const fs::path log = f.dir / "golden.log";
    ac::log::Init(log.string(), false);

    ac::log::Write(ac::LogLevel::Info, "Mod", "hello %d", 42);
    ac::log::WriteOnce(ac::LogLevel::Warn, "Once", "seen");
    ac::log::WriteOnce(ac::LogLevel::Warn, "Once", "seen");
    ac::log::Write(ac::LogLevel::Trace, "Mod", "filtered-by-level-gate");
    ac::log::Write(ac::LogLevel::Info, "Mod", "%s", std::string(2000, 'x').c_str());
    ac::log::Shutdown();

    const std::string body = f.Read(log);
    CHECK(body.find("[INFO ] [Mod] hello 42") != std::string::npos);
    CHECK(body.find("filtered-by-level-gate") == std::string::npos);
    CHECK(body.find("[...]") != std::string::npos); // truncation marker
    std::size_t seen = 0, pos = 0;                  // WriteOnce emitted exactly once
    while ((pos = body.find("[Once] seen", pos)) != std::string::npos) {
        ++seen;
        ++pos;
    }
    CHECK(seen == 1);
}

// W3: success requires a non-null remote module handle. Self-injection of an
// already-loaded system DLL is the real primitive on the current process
// (harmless: only bumps kernel32's refcount); the empty path is refused.
void RemoteInject() {
    HANDLE self = GetCurrentProcess();
    CHECK(ac::inject::RemoteLoadDll(self, L"kernel32.dll"));
    CHECK(!ac::inject::RemoteLoadDll(self, L""));
}

// B1: the async worker preserves every line under concurrent producers.
// Shutdown drains synchronously, so the file is complete when it returns.
void LoggerConcurrentBurst() {
    Fixture f;
    const fs::path log = f.dir / "burst.log";
    ac::log::Init(log.string(), false);

    constexpr int kThreads = 4, kPerThread = 250;
    std::vector<std::thread> threads;
    for (int t = 0; t < kThreads; ++t) {
        threads.emplace_back([t] {
            for (int i = 0; i < kPerThread; ++i)
                ac::log::Write(ac::LogLevel::Info, "Burst", "t=%d i=%d", t, i);
        });
    }
    for (auto& th : threads)
        th.join();
    ac::log::Shutdown();

    const std::string body = f.Read(log);
    std::size_t lines = 0, pos = 0;
    while ((pos = body.find("[Burst]", pos)) != std::string::npos) {
        ++lines;
        ++pos;
    }
    CHECK(lines == static_cast<std::size_t>(kThreads * kPerThread));
}

// Flush() (the same synchronous drain CrashDrain relies on) makes every
// line durable without stopping the worker: no Shutdown needed.
void LoggerFlushDrain() {
    Fixture f;
    const fs::path log = f.dir / "drain.log";
    ac::log::Init(log.string(), false);

    for (int i = 0; i < 100; ++i)
        ac::log::Write(ac::LogLevel::Info, "Drain", "line %d", i);
    ac::log::Flush();

    const std::string body = f.Read(log);
    std::size_t lines = 0, pos = 0;
    while ((pos = body.find("[Drain]", pos)) != std::string::npos) {
        ++lines;
        ++pos;
    }
    CHECK(lines == 100);
    ac::log::Shutdown();
}

// U1: strict UTF-8 <-> UTF-16 round-trips plus real filesystem ops inside
// a non-ASCII directory (the José case): every helper must work where *A
// APIs would mangle via the system codepage.
void UnicodePaths() {
    const std::string uni = "caf\xC3\xA9_\xE2\x98\x83"; // café_☃ in UTF-8
    CHECK(ac::strings::Narrow(ac::strings::Widen(uni)) == uni);
    CHECK(ac::strings::Widen("\xFF").empty()); // invalid UTF-8 fails closed
    // 6 UTF-16 units vs 9 UTF-8 bytes: decoded, not byte-copied.
    CHECK(ac::winfile::ToPath(uni).wstring().size() == 6);

    Fixture f;
    const std::string dir = ac::strings::Narrow((f.dir / "x").wstring()) + "_" + uni;
    CHECK(ac::winfile::CreateDir(dir));
    CHECK(ac::winfile::Exists(dir));
    CHECK(ac::winfile::IsDirectory(dir));
    const std::string file = dir + "\\payload.txt";
    {
        std::ofstream out(ac::winfile::ToPath(file), std::ios::binary);
        CHECK(out.is_open());
        out << "snowman";
    }
    const std::string moved = dir + "\\moved.txt";
    CHECK(ac::winfile::MoveReplace(file, moved));
    CHECK(!ac::winfile::Exists(file));
    const std::string copied = dir + "\\copied.txt";
    CHECK(ac::winfile::CopyOverwrite(moved, copied));
    CHECK(f.Read(ac::winfile::ToPath(moved)) == "snowman");
    CHECK(ac::winfile::Delete(copied));
    CHECK(!ac::winfile::Exists(copied));
}

int main(int argc, char** argv) {
    using Case = std::pair<const char*, void (*)()>;
    const Case cases[] = {
        {"staging_unique", StagingUnique},
        {"size_verify", SizeVerify},
        {"backup_rules", BackupRules},
        {"backup_concurrent_writers", BackupConcurrentWriters},
        {"pin_scan", PinScan},
        {"http_gate", HttpGate},
        {"logger_golden", LoggerGolden},
        {"logger_concurrent_burst", LoggerConcurrentBurst},
        {"logger_flush_drain", LoggerFlushDrain},
        {"quota_corrupt", QuotaCorrupt},
        {"restore_roundtrip", RestoreRoundtrip},
        {"manifest_index_backup_hit", ManifestIndexBackupHit},
        {"unicode_paths", UnicodePaths},
        {"remote_inject", RemoteInject},
    };
    try {
        if (argc == 1) { // run-all mode (ctest registers each case separately)
            for (const auto& [name, fn] : cases) {
                fn();
                std::cout << name << ": PASS\n";
            }
            return 0;
        }
        CHECK(argc == 2);
        const std::string test = argv[1];
        for (const auto& [name, fn] : cases) {
            if (test == name) {
                fn();
                std::cout << test << ": PASS\n";
                return 0;
            }
        }
        CHECK(false);
        return 1;
    } catch (const std::exception& e) {
        std::cerr << e.what() << '\n';
        return 1;
    }
}
