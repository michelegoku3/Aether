#include "pch.h"
#include "PayloadLog.h"

#include <windows.h>

#include <cstdio>
#include <ctime>
#include <filesystem>
#include <fstream>
#include <mutex>
#include <string>

namespace ac::payloadlog {
namespace {

// Payload DLL private logging context. The payload has no AetherCoreState
// lifetime, so this minimal per-DLL micro-state is an allowed exception.
std::mutex s_lock;
std::string s_path;

DWORD s_pid = 0;
char s_exeName[MAX_PATH] = {};
char s_steamAppId[32] = {};
char s_steamOverlayGameId[32] = {};

std::string ModuleDir(HMODULE self) {
    char path[MAX_PATH] = {};
    DWORD len = GetModuleFileNameA(self, path, MAX_PATH);
    if (len == 0 || len >= MAX_PATH) return {};
    std::string out(path, len);
    const std::size_t slash = out.find_last_of("\\/");
    return slash == std::string::npos ? out : out.substr(0, slash);
}

void FormatTimeAndThread(char (&out)[32]) {
    SYSTEMTIME st{};
    GetLocalTime(&st);
    const DWORD tid = GetCurrentThreadId();
    std::snprintf(out, sizeof(out), "%02d:%02d:%02d.%03d] [TID:%04lu",
                  st.wHour, st.wMinute, st.wSecond, st.wMilliseconds, static_cast<unsigned long>(tid));
}

void CaptureProcessIdentity() {
    s_pid = GetCurrentProcessId();

    char fullPath[MAX_PATH] = {};
    DWORD len = GetModuleFileNameA(nullptr, fullPath, MAX_PATH);
    if (len > 0 && len < MAX_PATH) {
        const char* name = fullPath;
        for (DWORD i = 0; i < len; ++i) {
            if (fullPath[i] == '\\' || fullPath[i] == '/') name = fullPath + i + 1;
        }
        std::snprintf(s_exeName, sizeof(s_exeName), "%s", name);
    } else {
        std::snprintf(s_exeName, sizeof(s_exeName), "?");
    }

    char buf[64] = {};
    if (GetEnvironmentVariableA("SteamAppId", buf, sizeof(buf))) {
        DWORD appId = static_cast<DWORD>(std::strtoul(buf, nullptr, 10));
        std::snprintf(s_steamAppId, sizeof(s_steamAppId), "%u", appId);
    } else {
        std::snprintf(s_steamAppId, sizeof(s_steamAppId), "-");
    }
    if (GetEnvironmentVariableA("SteamOverlayGameId", buf, sizeof(buf))) {
        std::snprintf(s_steamOverlayGameId, sizeof(s_steamOverlayGameId), "%s", buf);
    } else {
        std::snprintf(s_steamOverlayGameId, sizeof(s_steamOverlayGameId), "-");
    }
}

}  // namespace

void Init(HMODULE self) {
    std::lock_guard<std::mutex> lock(s_lock);
    const std::string dir = ModuleDir(self);
    if (dir.empty()) return;
    const std::string logDir = dir + "\\aethercore";
    CreateDirectoryA(logDir.c_str(), nullptr);
    // Directly target main.log so payload logs are funneled into the single main log.
    s_path = logDir + "\\main.log";
    CaptureProcessIdentity();
}

void Write(std::string_view line) {
    std::lock_guard<std::mutex> lock(s_lock);
    if (s_path.empty()) return;

    // Use an interprocess named mutex so game process and steam process write safely to main.log
    HANDLE hMutex = CreateMutexA(nullptr, FALSE, "Global\\AetherCore_Log_Mutex");
    if (hMutex) {
        WaitForSingleObject(hMutex, INFINITE);
    }

    std::ofstream out(s_path, std::ios::binary | std::ios::app);
    if (out.is_open()) {
        char timeAndThread[32];
        FormatTimeAndThread(timeAndThread);
        out << '[' << timeAndThread << "] [INFO ] [Payload] [exe=" << s_exeName
            << " pid=" << s_pid
            << " AppId=" << s_steamAppId << "] " << line << "\r\n";
        out.flush();
    }

    if (hMutex) {
        ReleaseMutex(hMutex);
        CloseHandle(hMutex);
    }
}

}  // namespace ac::payloadlog
