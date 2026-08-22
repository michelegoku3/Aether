#include "pch.h"
#include "hooks/wire/BackupIo.h"

#include <cstdio>
#include <ctime>
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

// Cache per processo del percorso AetherData (vedi CachedDeskDataDir).
std::mutex g_cacheMutex;
std::string g_cachedDeskData;
bool g_deskDataResolved = false;

std::string ReadDeskDataDirLocked() {
    std::ifstream ifs(g_state.aetherCoreDir + "\\desk_path.cfg");
    if (!ifs.is_open()) return {};
    std::string line;
    if (!std::getline(ifs, line)) return {};
    while (!line.empty() && (line.back() == '\r' || line.back() == '\n' || line.back() == ' ')) {
        line.pop_back();
    }
    return line;
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
    std::lock_guard<std::mutex> lock(g_cacheMutex);
    if (!g_deskDataResolved) {
        g_cachedDeskData = ReadDeskDataDirLocked();
        g_deskDataResolved = true;
        if (g_cachedDeskData.empty()) {
            AC_LOG_WARN_ONCE(kModule,
                             "Backup: AetherData path unknown (desk_path.cfg missing): "
                             "achievement backup disabled for this session.");
        }
    }
    return g_cachedDeskData;
}

std::string BackupDirForApp(steam::AppId appId) {
    const std::string deskData = CachedDeskDataDir();
    if (deskData.empty()) return {};
    const std::string app = std::to_string(appId);
    const std::string root = deskData + "\\backup";
    CreateDirectoryA(root.c_str(), nullptr);
    CreateDirectoryA((root + "\\" + app).c_str(), nullptr);
    const std::string dir = root + "\\" + app + "\\achievements";
    CreateDirectoryA(dir.c_str(), nullptr);
    return dir;
}

std::string BackupPlaytimeDir() {
    const std::string deskData = CachedDeskDataDir();
    if (deskData.empty()) return {};
    const std::string dir = deskData + "\\backup\\playtime";
    CreateDirectoryA(dir.c_str(), nullptr);
    return dir;
}

bool AtomicReplace(const std::string& tmp, const std::string& dst) {
    return MoveFileExA(tmp.c_str(), dst.c_str(), MOVEFILE_REPLACE_EXISTING) != FALSE;
}

}  // namespace ac::backup::io
