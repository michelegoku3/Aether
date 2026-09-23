#include "pch.h"
#include "hooks/wire/BackupIo.h"

#include <cstdio>
#include <ctime>
#include <filesystem>
#include <fstream>
#include <mutex>

#include "core/AetherCoreState.h"
#include "core/Logger.h"

namespace ac::backup::io {
namespace {

constexpr const char* kModule = "Wire.Achievement";

std::tm LocalTime(std::time_t tt) {
    std::tm tmBuf{};
#if defined(_WIN32)
    localtime_s(&tmBuf, &tt);
#else
    localtime_r(&tt, &tmBuf);
#endif
    return tmBuf;
}

}  // namespace

std::string FormatUnixTime(std::uint64_t unixTime) {
    const std::tm tmBuf = LocalTime(static_cast<std::time_t>(unixTime));
    char buf[40];
    std::snprintf(buf, sizeof(buf), "%02d/%02d/%04d %02d:%02d:%02d",
                  tmBuf.tm_mday, tmBuf.tm_mon + 1, tmBuf.tm_year + 1900,
                  tmBuf.tm_hour, tmBuf.tm_min, tmBuf.tm_sec);
    return buf;
}

std::string FormatWallClockNow() {
    const std::tm tmBuf = LocalTime(std::time(nullptr));
    char buf[32];
    std::snprintf(buf, sizeof(buf), "%04d-%02d-%02dT%02d:%02d:%02d",
                  tmBuf.tm_year + 1900, tmBuf.tm_mon + 1, tmBuf.tm_mday,
                  tmBuf.tm_hour, tmBuf.tm_min, tmBuf.tm_sec);
    return buf;
}

std::string CachedDeskDataDir() {
    // Resolved before any backup worker is started; immutable for this session.
    return g_state.deskDataDir;
}

std::string BackupDirForApp(steam::AppId appId) {
    const std::string deskData = CachedDeskDataDir();
    if (deskData.empty()) return {};
    const std::filesystem::path dir = std::filesystem::path(deskData) / "backup" /
                                      std::to_string(appId) / "achievements";
    std::error_code ec;
    if (!std::filesystem::create_directories(dir, ec) && ec) return {};
    return dir.string();
}

std::string BackupPlaytimeDir() {
    const std::string deskData = CachedDeskDataDir();
    if (deskData.empty()) return {};
    const std::filesystem::path dir = std::filesystem::path(deskData) / "backup" / "playtime";
    std::error_code ec;
    if (!std::filesystem::create_directories(dir, ec) && ec) return {};
    return dir.string();
}

bool AtomicReplace(const std::string& tmp, const std::string& dst) {
    return MoveFileExA(tmp.c_str(), dst.c_str(), MOVEFILE_REPLACE_EXISTING) != FALSE;
}

}  // namespace ac::backup::io
