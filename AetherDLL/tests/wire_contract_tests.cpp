#include "pch.h"

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "core/Settings.h"
#include "hooks/wire/BackupIo.h"
#include "hooks/wire/ManifestRestore.h"
#include "network/RuntimeHttp.h"
#include "utils/RemoteInject.h"

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
