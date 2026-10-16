#include "pch.h"
#include "core/Constants.h"
#include "core/Logger.h"

#include <windows.h>

#include <array>
#include <atomic>
#include <cctype>
#include <chrono>
#include <condition_variable>
#include <cstdarg>
#include <cstdio>
#include <ctime>
#include <deque>
#include <exception>
#include <filesystem>
#include <fstream>
#include <mutex>
#include <string>
#include <thread>
#include <unordered_set>
#include <utility>
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
    case LogLevel::Trace:
        return "TRACE";
    case LogLevel::Debug:
        return "DEBUG";
    case LogLevel::Info:
        return "INFO ";
    case LogLevel::Warn:
        return "WARN ";
    case LogLevel::Error:
        return "ERROR";
    default:
        return "?????";
    }
}

// Formats timestamp with millisecond precision and Thread ID.
void FormatTimeAndThread(char (&out)[32]) {
    SYSTEMTIME st{};
    GetLocalTime(&st);
    const DWORD tid = GetCurrentThreadId();
    // Format is provably bounded (~28 chars into 32): truncation cannot occur.
    (void)std::snprintf(out, sizeof(out), "%02d:%02d:%02d.%03d] [TID:%04lu", st.wHour, st.wMinute, st.wSecond,
                        st.wMilliseconds, static_cast<unsigned long>(tid));
}

// Formats a body with truncation marker. Shared by Write/WriteOnce so the
// policy lives in one place.
void FormatBody(char (&body)[constants::kLogBodyBytes], const char* format, va_list args) {
    const int written = std::vsnprintf(body, sizeof(body), format, args);
    if (written >= 0 && static_cast<std::size_t>(written) >= sizeof(body)) {
        const char marker[] = "[...]";
        const std::size_t mlen = sizeof(marker) - 1;
        if (sizeof(body) > mlen) {
            for (std::size_t i = 0; i < mlen; ++i)
                body[sizeof(body) - mlen - 1 + i] = marker[i];
        }
    }
}

// ---------------------------------------------------------------------------
// Async publish path.
//
// Producers (any Steam thread, including the packet/IPC hot paths) only
// format the line and push it on a queue: no kernel calls, no disk I/O, no
// cross-process mutex on the calling thread. A single dedicated worker owns
// the file handle, the cross-process mutex and the flush policy, so output
// order is FIFO and identical bytes are emitted for identical inputs.
//
// Flush policy: EVERY line is flushed by the worker immediately after it is
// written. Crash forensics demands it: the last lines before a crash are
// exactly the ones that explain it, so no batching window is acceptable.
// This costs the hook threads nothing (they only enqueue); the disk cost
// lives in the worker, same IOPS as the historical synchronous logger.
// The only residual window is queue-to-worker latency (sub-millisecond),
// closed for catchable crashes by CrashDrain() below.
// ---------------------------------------------------------------------------
struct LogItem {
    LogLevel level = LogLevel::Info;
    std::string text; // fully formatted line, timestamp included
};

constexpr auto kDrainTimeout = std::chrono::seconds(5);

struct LogWorker {
    std::mutex mutex;
    std::condition_variable cv;
    std::deque<LogItem> queue;
    std::thread thread;
    bool running = false;
    bool stop = false;
    HANDLE hMutex = nullptr; // cached cross-process mutex (opened once in Init)
    // Durability accounting: enqueued counts pushed lines, completed counts
    // lines written AND flushed. Flush() waits for completed >= target, so an
    // empty queue alone (last item popped but not yet on disk) is never
    // mistaken for "all durable". Both advance under mutex; loads are atomic.
    std::atomic<std::uint64_t> enqueued{0};
    std::atomic<std::uint64_t> completed{0};
};

// Pinned-leak on purpose (same pattern as StatusWriter's worker): the DLL is
// pinned via PinSelf, so a joining std::thread destructor at CRT detach must
// never run. Stop/Join happen only in the explicit off-loader-lock Shutdown.
LogWorker* s_worker = nullptr;
std::mutex s_lifeMutex; // guards s_worker pointer only (never held on hot path)

void WriteLineLocked(LogWorker* w, const LogItem& item) {
    if (!s_file.is_open() && !s_logPath.empty()) {
        s_file.open(s_logPath, std::ios::out | std::ios::app);
    }
    if (!s_file.is_open())
        return;
    if (w->hMutex) {
        WaitForSingleObject(w->hMutex, INFINITE);
    }
    s_file << item.text << '\n';
    // Per-line flush, unconditionally: the worker owns the disk cost, and a
    // process crash must never lose an already-written line. (level is kept
    // on the item for future filtering; flushing no longer depends on it.)
    s_file.flush();
    if (w->hMutex) {
        ReleaseMutex(w->hMutex);
    }
}

void WorkerLoop(LogWorker* w) {
    for (;;) {
        LogItem item;
        {
            std::unique_lock<std::mutex> lock(w->mutex);
            w->cv.wait(lock, [&] { return w->stop || !w->queue.empty(); });
            if (w->stop && w->queue.empty())
                break;
            item = std::move(w->queue.front());
            w->queue.pop_front();
        }
        {
            std::lock_guard<std::mutex> fileLock(s_mutex);
            WriteLineLocked(w, item);
        }
        w->completed.fetch_add(1, std::memory_order_release);
    }
    std::lock_guard<std::mutex> fileLock(s_mutex);
    if (s_file.is_open())
        s_file.flush();
}

// ---------------------------------------------------------------------------
// Crash drain: zero-loss forensics for catchable process deaths.
//
// Even with per-line worker flushes, lines already enqueued but not yet
// popped by the worker live only in memory. When the process is dying from
// any catchable exception, CrashDrain() writes them synchronously before
// the crash flow continues, so the log ends at the faulting instruction.
//
// Coverage ("all exceptions", best-effort):
//   * VEH (first): every hardware fault (AV, div-by-zero, illegal
//     instruction, stack overflow, breakpoints). VEH also fires for
//     dispatched-then-caught exceptions; the drain is idempotent and cheap
//     when the queue is empty, so that is harmless.
//   * UEF: unhandled SEH and escaped C++ exceptions. NOTE: if the host
//     installs its own UEF after ours, theirs wins; the VEH still runs, so
//     hardware faults stay covered either way.
//   * terminate handler: noexcept violations, throws from destructors,
//     direct std::terminate. Re-invokes the previous handler afterwards.
// Not coverable by design: TerminateProcess/kill, power loss (only
// FILE_FLAG_WRITE_THROUGH would close that; rejected for its 10-100x disk
// cost), or corruption so deep the queue itself is unreadable.
//
// Deadlock discipline: try_lock only, never lock. If the fault hit while a
// thread held a logger mutex, blocking on it would hang the crash report.
// An atomic reentrancy guard bails out of nested invocation. Handlers never
// swallow: they drain, then let the normal crash flow (dumps, WER) proceed.
// ---------------------------------------------------------------------------
std::atomic<bool> s_crashDraining{false};
void* s_vehHandle = nullptr;
LPTOP_LEVEL_EXCEPTION_FILTER s_prevUef = nullptr;
std::terminate_handler s_prevTerminate = nullptr;
bool s_crashHandlersInstalled = false; // under s_lifeMutex, like s_worker

void CrashDrain() {
    if (s_crashDraining.exchange(true, std::memory_order_acq_rel))
        return; // nested crash inside the drain: do not hang the report
    LogWorker* w = nullptr;
    {
        std::lock_guard<std::mutex> life(s_lifeMutex);
        w = s_worker;
    }
    if (w) {
        // Best-effort: pop what we can without blocking.
        for (;;) {
            LogItem item;
            {
                std::unique_lock<std::mutex> qlock(w->mutex, std::try_to_lock);
                if (!qlock.owns_lock() || w->queue.empty())
                    break;
                item = std::move(w->queue.front());
                w->queue.pop_front();
            }
            std::unique_lock<std::mutex> flock(s_mutex, std::try_to_lock);
            if (!flock.owns_lock())
                break; // file busy at fault time; already-written lines were flushed per-line anyway
            WriteLineLocked(w, item);
        }
    }
    {
        std::unique_lock<std::mutex> flock(s_mutex, std::try_to_lock);
        if (flock.owns_lock() && s_file.is_open())
            s_file.flush();
    }
    s_crashDraining.store(false, std::memory_order_release);
}

LONG WINAPI CrashVectoredHandler(EXCEPTION_POINTERS*) {
    CrashDrain();
    return EXCEPTION_CONTINUE_SEARCH; // never swallow: reports/dumps proceed normally
}

LONG WINAPI CrashUnhandledFilter(EXCEPTION_POINTERS*) {
    CrashDrain();
    return EXCEPTION_CONTINUE_SEARCH; // keep default termination semantics
}

void CrashTerminateHandler() {
    CrashDrain();
    const auto prev = s_prevTerminate;
    if (prev && prev != &CrashTerminateHandler)
        prev();
    std::abort();
}

void InstallCrashHandlers() {
    if (s_crashHandlersInstalled)
        return;
    s_vehHandle = AddVectoredExceptionHandler(1, CrashVectoredHandler);
    s_prevUef = SetUnhandledExceptionFilter(CrashUnhandledFilter);
    s_prevTerminate = std::set_terminate(CrashTerminateHandler);
    s_crashHandlersInstalled = true;
}

void RemoveCrashHandlers() {
    if (!s_crashHandlersInstalled)
        return;
    if (s_vehHandle) {
        RemoveVectoredExceptionHandler(s_vehHandle);
        s_vehHandle = nullptr;
    }
    SetUnhandledExceptionFilter(s_prevUef);
    s_prevUef = nullptr;
    std::set_terminate(s_prevTerminate != &CrashTerminateHandler ? s_prevTerminate : nullptr);
    s_prevTerminate = nullptr;
    s_crashHandlersInstalled = false;
}

// Synchronous fallback: used before Init and after Shutdown (and when the
// queue is unavailable). Preserves the historical behavior of emitting
// directly, including the lazy reopen.
void EmitSync(LogLevel level, const char* module, const char* body) {
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
        s_file << '[' << timeAndThread << "] [" << LevelTag(level) << "] [" << (module ? module : "-") << "] " << body
               << '\n';
        s_file.flush();
    }

    if (hMutex) {
        ReleaseMutex(hMutex);
        CloseHandle(hMutex);
    }
}

// Single publish path. Write/WriteOnce differ only in formatting gate and
// dedup; the emitted bytes are identical for identical inputs.
void Emit(LogLevel level, const char* module, const char* body) {
    char timeAndThread[32];
    FormatTimeAndThread(timeAndThread);

    std::string line;
    try {
        line.reserve(128);
        line += '[';
        line += timeAndThread;
        line += "] [";
        line += LevelTag(level);
        line += "] [";
        line += (module ? module : "-");
        line += "] ";
        line += body;
    } catch (...) {
        return; // best-effort logger: never throw to hook threads
    }

    LogWorker* w = nullptr;
    {
        std::lock_guard<std::mutex> life(s_lifeMutex);
        w = s_worker;
    }
    if (!w) {
        EmitSync(level, module, body);
        return;
    }
    bool enqueued = false;
    try {
        std::lock_guard<std::mutex> lock(w->mutex);
        // When the worker is draining, fall back to synchronous emit so
        // nothing logged during shutdown is silently lost.
        if (w->running && !w->stop) {
            w->queue.push_back(LogItem{level, std::move(line)});
            w->enqueued.fetch_add(1, std::memory_order_release);
            enqueued = true;
        }
    } catch (...) {
        return;
    }
    if (enqueued) {
        w->cv.notify_one();
    } else {
        EmitSync(level, module, body);
    }
}

} // namespace

void Init(const std::string& filePath, bool keepLastSession) {
    // Restart-safe: tests call Init once per fixture, so drain any previous
    // worker before replacing the session.
    Shutdown();

    std::lock_guard<std::mutex> lock(s_mutex);
    s_logPath = filePath;

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

    LogWorker* w = nullptr;
    try {
        w = new LogWorker();
        w->hMutex = CreateMutexA(nullptr, FALSE, "Global\\AetherCore_Log_Mutex");
        w->thread = std::thread(WorkerLoop, w);
        w->running = true;
    } catch (...) {
        if (w) {
            if (w->hMutex)
                CloseHandle(w->hMutex);
            delete w;
        }
        return; // producers fall back to EmitSync; no throw to the caller
    }
    {
        std::lock_guard<std::mutex> life(s_lifeMutex);
        s_worker = w;
        // Crash forensics (VEH first, then UEF, then terminate): the drain
        // must be armed before the first hook can fault.
        InstallCrashHandlers();
    }
}

bool IsEnabled(LogLevel level) {
    return static_cast<int>(level) >= static_cast<int>(s_minLevel.load(std::memory_order_relaxed));
}

void SetLevel(LogLevel level) { s_minLevel.store(level, std::memory_order_relaxed); }

LogLevel ParseLevel(const std::string& text, LogLevel fallback) {
    std::string lower;
    lower.reserve(text.size());
    for (char c : text)
        lower.push_back(static_cast<char>(std::tolower(static_cast<unsigned char>(c))));

    if (lower == "trace")
        return LogLevel::Trace;
    if (lower == "debug")
        return LogLevel::Debug;
    if (lower == "info")
        return LogLevel::Info;
    if (lower == "warn")
        return LogLevel::Warn;
    if (lower == "error")
        return LogLevel::Error;
    if (lower == "off")
        return LogLevel::Off;
    return fallback;
}

void Write(LogLevel level, const char* module, const char* format, ...) {
    // Early-out: avoid formatting for filtered messages.
    if (static_cast<int>(level) < static_cast<int>(s_minLevel.load(std::memory_order_relaxed))) {
        return;
    }

    char body[constants::kLogBodyBytes];
    va_list args;
    va_start(args, format);
    FormatBody(body, format, args);
    va_end(args);

    Emit(level, module, body);
}

void WriteOnce(LogLevel level, const char* module, const char* format, ...) {
    // Early-out: avoid formatting for filtered messages.
    if (static_cast<int>(level) < static_cast<int>(s_minLevel.load(std::memory_order_relaxed))) {
        return;
    }

    char body[constants::kLogBodyBytes];
    va_list args;
    va_start(args, format);
    FormatBody(body, format, args);
    va_end(args);

    // Build dedup key: "module|body". If already seen this session, drop.
    std::string key;
    key.reserve(64);
    key.append(module ? module : "-");
    key.push_back('|');
    key.append(body);

    {
        std::lock_guard<std::mutex> dlock(s_dedupMutex);
        if (!s_dedupSeen.insert(key).second) {
            return; // Already emitted this session.
        }
    }

    Emit(level, module, body);
}

void ResetDedup() {
    std::lock_guard<std::mutex> lock(s_dedupMutex);
    s_dedupSeen.clear();
}

// Drains the queue synchronously (bounded): after Flush returns, every line
// enqueued before the call is written AND flushed. NEVER call from DllMain:
// it waits on the worker thread. An empty queue is NOT sufficient (the last
// popped item may not be on disk yet); durability is tracked with the
// completed counter instead.
void Flush() {
    LogWorker* w = nullptr;
    {
        std::lock_guard<std::mutex> life(s_lifeMutex);
        w = s_worker;
    }
    if (!w)
        return;
    const std::uint64_t target = w->enqueued.load(std::memory_order_acquire);
    const auto deadline = std::chrono::steady_clock::now() + kDrainTimeout;
    while (w->completed.load(std::memory_order_acquire) < target) {
        if (std::chrono::steady_clock::now() >= deadline)
            break;
        std::this_thread::sleep_for(std::chrono::milliseconds(1));
    }
    std::lock_guard<std::mutex> lock(s_mutex);
    if (s_file.is_open())
        s_file.flush();
}

void Shutdown() {
    LogWorker* w = nullptr;
    {
        std::lock_guard<std::mutex> life(s_lifeMutex);
        w = s_worker;
        s_worker = nullptr;
        RemoveCrashHandlers();
    }
    if (w) {
        // Explicit off-loader-lock shutdown only: joins the worker thread.
        // Never from DllMain (loader lock); DllMain DETACH only sets the
        // global shutdown flag and never touches the logger.
        {
            std::lock_guard<std::mutex> lock(w->mutex);
            w->stop = true;
        }
        w->cv.notify_all();
        if (w->thread.joinable())
            w->thread.join();
        std::lock_guard<std::mutex> lock(s_mutex);
        if (s_file.is_open()) {
            s_file.flush();
            s_file.close();
        }
        if (w->hMutex)
            CloseHandle(w->hMutex);
        delete w;
        return;
    }
    std::lock_guard<std::mutex> lock(s_mutex);
    if (s_file.is_open())
        s_file.close();
}

} // namespace ac::log

namespace ac::diag {
namespace {

// Diagnostics ring owned by the diagnostics service. StatusWriter consumes it
// through Snapshot(); feature/domain counters still live in AetherCoreState.
constexpr std::size_t kRingSize = 64;
std::array<Entry, kRingSize> s_ring{};
std::size_t s_next = 0;
std::size_t s_count = 0;
std::mutex s_diagMutex;

} // namespace

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
    if (s_count < kRingSize)
        ++s_count;
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

} // namespace ac::diag
