#include "pch.h"
#include "core/Workers.h"
#include "core/Logger.h"
#include "hooks/wire/AchievementBackup.h"
#include "hooks/wire/BackupIo.h"
#include "hooks/wire/PlaytimeMirror.h"
#include "hooks/wire/SteamStatsCache.h"
#include "hooks/wire/UserStatsSnapshot.h"
#include <atomic>
#include <chrono>
#include <iostream>
#include <stdexcept>
#include <thread>
using namespace std::chrono_literals;
#define CHECK(x) do { if (!(x)) throw std::runtime_error(#x); } while (false)
namespace {
std::atomic<int> saves{0}, copies{0}, refreshes{0};
std::atomic<bool> failNextSave{false};
}
namespace ac::log {
void Write(LogLevel, const char*, const char*, ...) {}
void WriteOnce(LogLevel, const char*, const char*, ...) {}
}
namespace ac::backup::io {
std::string FormatUnixTime(std::uint64_t n) { return std::to_string(n); }
}
namespace ac::backup::snapshot {
std::string SnapshotPath(steam::AppId, std::uint32_t) { return "test-snapshot"; }
SnapshotData Load(const std::string&) { return {}; }
void Save(const std::string&, steam::AppId, std::uint32_t, std::uint64_t, const SnapshotData&) {
    if (failNextSave.exchange(false)) throw std::runtime_error("simulated I/O exception");
    ++saves;
}
}
namespace ac::backup::statscache {
void BackupStatsBins(steam::AppId, std::uint32_t, bool) { ++copies; }
const std::unordered_set<std::uint32_t>& SchemaBucketsFor(steam::AppId) {
    static const std::unordered_set<std::uint32_t> empty;
    return empty;
}
}
namespace ac::backup::playtime {
void RefreshAllAccounts() { ++refreshes; }
}
int main(int argc, char** argv) {
    namespace b = ac::hooks::AchievementBackup;
    namespace w = ac::workers;
    try {
        const std::string mode = argc > 1 ? argv[1] : "registry_first";
        if (mode == "registration_refused") {
            w::Shutdown();
            b::RecordUnlock(10, 20, 30, 40);
            b::FlushOnShutdown();
            CHECK(saves == 0 && copies == 0 && refreshes == 0);
            return 0;
        }
        if (mode == "startup_scan") {
            backup_test::scanFile = true;
            b::BackupAllKnownStatsAtStartup();
            w::Shutdown();
            b::FlushOnShutdown();
            CHECK(copies == 2 && refreshes == 2); // startup + final
            return 0;
        }
        const bool failure = mode == "job_failure";
        failNextSave = failure;
        b::RecordUnlock(10, 20, 30, 40);
        b::RecordStats(10, 20, {{1, 2}});
        b::SessionEnded(10, 20); // due in 15s: shutdown must drain it immediately
        if (mode == "idle_registry") {
            const auto deadline = std::chrono::steady_clock::now() + 5s;
            while (saves < 2 && std::chrono::steady_clock::now() < deadline)
                std::this_thread::sleep_for(1ms);
            CHECK(saves == 2);
            std::this_thread::sleep_for(30ms);
            CHECK(copies == 1); // 15s job is still deferred before shutdown
        }
        const auto start = std::chrono::steady_clock::now();
        if (mode == "flush_first") {
            b::FlushOnShutdown();
            CHECK(saves == 2 && copies == 3 && refreshes == 1);
            w::Shutdown();
        } else if (mode == "concurrent") {
            std::thread shutdown([] { w::Shutdown(); });
            b::FlushOnShutdown();
            shutdown.join();
        } else {
            w::Shutdown();
            b::FlushOnShutdown();
        }
        CHECK(std::chrono::steady_clock::now() - start < 5s);
        CHECK(saves == (failure ? 1 : 2));
        CHECK(copies == (failure ? 2 : 3)); // unlock + delayed copy + final copy
        CHECK(refreshes == 1);
        const int before = saves;
        b::RecordUnlock(10, 20, 31, 41); // rejected, never resurrects the worker
        b::FlushOnShutdown();
        CHECK(saves == before);
        w::Shutdown();
        std::cout << mode << " PASS\n";
        return 0;
    } catch (const std::exception& e) {
        std::cerr << e.what() << '\n';
        w::Shutdown();
        return 1;
    }
}
