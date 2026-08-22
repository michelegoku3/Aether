#include "pch.h"
#include "core/Logger.h"

#include <windows.h>

#include <array>
#include <atomic>
#include <cctype>
#include <cstdarg>
#include <cstdio>
#include <ctime>
#include <filesystem>
#include <fstream>
#include <mutex>
#include <string>
#include <unordered_set>
#include <vector>

namespace ac::log {
namespace {

// Logger-owned process-local service state. It is intentionally module-local:
// callers interact through the logger API, and no AetherCore domain state is
// stored here. See docs/ARCHITECTURE.md.
std::ofstream s_file;
std::mutex s_mutex;
std::atomic<LogLevel> s_minLevel{LogLevel::Info};
std::string s_logPath;

// Per-session deduplication set. Keyed on "module|body" so the same formatted
// text in different modules is treated independently. Cleared by ResetDedup()
// when a new game session begins.
std::mutex s_dedupMutex;
std::unordered_set<std::string> s_dedupSeen;

const char* LevelTag(LogLevel level) {
    switch (level) {
        case LogLevel::Trace: return "TRACE";
        case LogLevel::Debug: return "DEBUG";
        case LogLevel::Info:  return "INFO ";
        case LogLevel::Warn:  return "WARN ";
        case LogLevel::Error: return "ERROR";
        default:              return "?????";
    }
}

// Formats timestamp with millisecond precision and Thread ID.
void FormatTimeAndThread(char (&out)[32]) {
    SYSTEMTIME st{};
    GetLocalTime(&st);
    const DWORD tid = GetCurrentThreadId();
    std::snprintf(out, sizeof(out), "%02d:%02d:%02d.%03d] [TID:%04lu",
                  st.wHour, st.wMinute, st.wSecond, st.wMilliseconds, static_cast<unsigned long>(tid));
}

}  // namespace

void Init(const std::string& filePath, bool keepLastSession) {
    std::lock_guard<std::mutex> lock(s_mutex);
    s_logPath = filePath;

    if (s_file.is_open()) {
        s_file.close();
    }

    if (keepLastSession && !s_logPath.empty()) {
        std::error_code ec;
        if (std::filesystem::exists(s_logPath, ec)) {
            const std::string backupPath = s_logPath + ".last";
            std::filesystem::remove(backupPath, ec);
            ec.clear();
            std::filesystem::rename(s_logPath, backupPath, ec);
        }
    }

    s_file.open(filePath, std::ios::out | std::ios::trunc);
}

bool IsEnabled(LogLevel level) {
    return static_cast<int>(level) >= static_cast<int>(s_minLevel.load(std::memory_order_relaxed));
}

void SetLevel(LogLevel level) {
    s_minLevel.store(level, std::memory_order_relaxed);
}

LogLevel ParseLevel(const std::string& text, LogLevel fallback) {
    std::string lower;
    lower.reserve(text.size());
    for (char c : text) lower.push_back(static_cast<char>(std::tolower(static_cast<unsigned char>(c))));

    if (lower == "trace") return LogLevel::Trace;
    if (lower == "debug") return LogLevel::Debug;
    if (lower == "info")  return LogLevel::Info;
    if (lower == "warn")  return LogLevel::Warn;
    if (lower == "error") return LogLevel::Error;
    if (lower == "off")   return LogLevel::Off;
    return fallback;
}

void Write(LogLevel level, const char* module, const char* format, ...) {
    // Early-out: avoid formatting for filtered messages.
    if (static_cast<int>(level) < static_cast<int>(s_minLevel.load(std::memory_order_relaxed))) {
        return;
    }

    char body[1024];
    va_list args;
    va_start(args, format);
    int written = std::vsnprintf(body, sizeof(body), format, args);
    va_end(args);

    if (written >= 0 && static_cast<std::size_t>(written) >= sizeof(body)) {
        const char marker[] = "[...]";
        const std::size_t mlen = sizeof(marker) - 1;
        if (sizeof(body) > mlen) {
            for (std::size_t i = 0; i < mlen; ++i)
                body[sizeof(body) - mlen - 1 + i] = marker[i];
        }
    }

    char timeAndThread[32];
    FormatTimeAndThread(timeAndThread);

    std::lock_guard<std::mutex> lock(s_mutex);

    // Synchronize across processes (Steam process & Game processes)
    HANDLE hMutex = CreateMutexA(nullptr, FALSE, "Global\\AetherCore_Log_Mutex");
    if (hMutex) {
        WaitForSingleObject(hMutex, INFINITE);
    }

    if (!s_file.is_open() && !s_logPath.empty()) {
        s_file.open(s_logPath, std::ios::out | std::ios::app);
    }
    if (s_file.is_open()) {
        s_file << '[' << timeAndThread << "] [" << LevelTag(level) << "] ["
               << (module ? module : "-") << "] " << body << '\n';
        s_file.flush();
    }

    if (hMutex) {
        ReleaseMutex(hMutex);
        CloseHandle(hMutex);
    }
}

void WriteOnce(LogLevel level, const char* module, const char* format, ...) {
    // Early-out: avoid formatting for filtered messages.
    if (static_cast<int>(level) < static_cast<int>(s_minLevel.load(std::memory_order_relaxed))) {
        return;
    }

    char body[1024];
    va_list args;
    va_start(args, format);
    int written = std::vsnprintf(body, sizeof(body), format, args);
    va_end(args);

    if (written >= 0 && static_cast<std::size_t>(written) >= sizeof(body)) {
        const char marker[] = "[...]";
        const std::size_t mlen = sizeof(marker) - 1;
        if (sizeof(body) > mlen) {
            for (std::size_t i = 0; i < mlen; ++i)
                body[sizeof(body) - mlen - 1 + i] = marker[i];
        }
    }

    // Build dedup key: "module|body". If already seen this session, drop.
    std::string key;
    key.reserve(64);
    key.append(module ? module : "-");
    key.push_back('|');
    key.append(body);

    {
        std::lock_guard<std::mutex> dlock(s_dedupMutex);
        if (!s_dedupSeen.insert(key).second) {
            return;  // Already emitted this session.
        }
    }

    char timeAndThread[32];
    FormatTimeAndThread(timeAndThread);

    std::lock_guard<std::mutex> lock(s_mutex);

    HANDLE hMutex = CreateMutexA(nullptr, FALSE, "Global\\AetherCore_Log_Mutex");
    if (hMutex) {
        WaitForSingleObject(hMutex, INFINITE);
    }

    if (!s_file.is_open() && !s_logPath.empty()) {
        s_file.open(s_logPath, std::ios::out | std::ios::app);
    }
    if (s_file.is_open()) {
        s_file << '[' << timeAndThread << "] [" << LevelTag(level) << "] ["
               << (module ? module : "-") << "] " << body << '\n';
        s_file.flush();
    }

    if (hMutex) {
        ReleaseMutex(hMutex);
        CloseHandle(hMutex);
    }
}

void ResetDedup() {
    std::lock_guard<std::mutex> lock(s_dedupMutex);
    s_dedupSeen.clear();
}

void Shutdown() {
    std::lock_guard<std::mutex> lock(s_mutex);
    if (s_file.is_open()) s_file.close();
}

void Flush() {
    std::lock_guard<std::mutex> lock(s_mutex);
    if (s_file.is_open()) s_file.flush();
}

}  // namespace ac::log

namespace ac::diag {
namespace {

// Diagnostics ring owned by the diagnostics service. StatusWriter consumes it
// through Snapshot(); feature/domain counters still live in AetherCoreState.
constexpr std::size_t kRingSize = 64;
std::array<Entry, kRingSize> s_ring{};
std::size_t s_next = 0;
std::size_t s_count = 0;
std::mutex s_diagMutex;

}  // namespace

void Record(const std::string& category, const std::string& detail) {
    std::lock_guard<std::mutex> lock(s_diagMutex);
    Entry& e = s_ring[s_next];

    FILETIME ft;
    GetSystemTimeAsFileTime(&ft);
    ULARGE_INTEGER ull;
    ull.LowPart = ft.dwLowDateTime;
    ull.HighPart = ft.dwHighDateTime;
    // Convert 100-nanosecond intervals to milliseconds since Unix epoch
    e.timestampMs = (ull.QuadPart / 10000ULL) - 11644473600000ULL;
    e.category = category;
    e.detail = detail;
    s_next = (s_next + 1) % kRingSize;
    if (s_count < kRingSize) ++s_count;
}

std::vector<Entry> Snapshot() {
    std::lock_guard<std::mutex> lock(s_diagMutex);
    std::vector<Entry> out;
    out.reserve(s_count);
    const std::size_t start = (s_next + kRingSize - s_count) % kRingSize;
    for (std::size_t i = 0; i < s_count; ++i) {
        out.push_back(s_ring[(start + i) % kRingSize]);
    }
    return out;
}

}  // namespace ac::diag
