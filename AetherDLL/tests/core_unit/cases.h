#pragma once

// Shared test-support header for the core unit suites: CHECK macro, temp-dir
// Fixture and forward declarations. Production headers stay in the per-area
// translation units so each suite documents its own dependencies.

#include "pch.h"

#include <atomic>
#include <chrono>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <random>
#include <sstream>
#include <stdexcept>
#include <thread>
#include <unordered_map>
#include <utility>
#include <vector>

#define CHECK(expr)                                                                                    \
    do {                                                                                               \
        if (!(expr))                                                                                   \
            throw std::runtime_error("Check failed at line " + std::to_string(__LINE__) + ": " #expr); \
    } while (false)
// CHECK_EQ prints both values when they differ (CHECK shows only the
// expression text, forcing a debugger re-run on every value mismatch).
// Non-streamable types still work: values are shown only when printable.
// No framework dependency: same throw contract as CHECK.
namespace testdetail {
template <typename T> void AppendValue(std::ostringstream& oss, const T& v) {
    if constexpr (requires { std::declval<std::ostringstream&>() << std::declval<const T&>(); }) {
        oss << v;
    } else {
        oss << "<unprintable>";
    }
}
template <typename A, typename B> void CheckEqImpl(const A& a, const B& b, int line, const char* expr) {
    if (!(a == b)) {
        std::ostringstream oss;
        oss << "Check failed at line " << line << ": " << expr << " (actual=";
        AppendValue(oss, a);
        oss << " expected=";
        AppendValue(oss, b);
        oss << ")";
        throw std::runtime_error(oss.str());
    }
}
} // namespace testdetail
#define CHECK_EQ(a, b) ::testdetail::CheckEqImpl((a), (b), __LINE__, #a " == " #b)
namespace fs = std::filesystem;
using namespace std::chrono_literals;
struct Fixture {
    fs::path dir = fs::temp_directory_path() /
                   ("aether-core-unit-" + std::to_string(std::chrono::steady_clock::now().time_since_epoch().count()));
    int revision = 0;
    Fixture() { fs::create_directories(dir); }
    ~Fixture() {
        std::error_code ec;
        fs::remove_all(dir, ec);
    }
    fs::path Write(const char* name, const std::string& data) {
        const auto path = dir / name;
        {
            std::ofstream f(path, std::ios::binary);
            f << data;
        }
        fs::last_write_time(path, fs::file_time_type::clock::now() + std::chrono::seconds(++revision));
        return path;
    }
};

void Strings();
void Paths();
void Policy();
void Worker();
void Budget();
void Settings();
void Workers();
void WorkersReapSlow();
void WorkersShutdownRace();
void SchemaBuckets();
void Registry();
void NetPacket();
void Sentinel();
void StructGuard();
void GamesPlayedFormatTest();
void DonorPoolTest();
void KeyValuesTest();
void VdfTextTest();
void JsonEscapesTest();
void ManifestIdentityTest();
void IpcReplyTest();
void SnapshotMergeTest();
void TtlCacheTest();
void HexCodecTest();
void SignatureTest();
void IpcSpecParseTest();
void JsonWriterTest();
void ManifestContractTest();
void AcfOwnerTest();
