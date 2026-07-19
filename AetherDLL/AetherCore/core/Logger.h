#pragma once

#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

// ---------------------------------------------------------------------------
// Lightweight, session-oriented levelled logger.
//
// Replaced size-based mid-execution file rotation with clean session management:
//   * On startup, backs up the previous main.log to main.log.last and opens a clean log.
//   * Output lines include high-precision timestamps (ms) and Thread IDs (TID):
//     [HH:MM:SS.mmm] [TID:1234] [INFO ] [Module] message
//
// All public functions are thread-safe and never throw.
// ---------------------------------------------------------------------------
namespace ac {

enum class LogLevel : int {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
    Off = 5,  // Disables all output; useful for tests.
};

namespace log {

// Opens (or creates) the log file for the session.
// If keepLastSession is true, renames the existing log to <filePath>.last.
void Init(const std::string& filePath, bool keepLastSession = true);

// Adjusts the minimum level emitted. Messages below this are dropped cheaply.
void SetLevel(LogLevel level);

// Parses "trace"/"debug"/"info"/"warn"/"error"/"off" (case-insensitive).
// Unknown strings fall back to the provided default.
LogLevel ParseLevel(const std::string& text, LogLevel fallback);

// Core entry point. Prefer the AC_LOG_* macros below so the module tag and
// level are passed consistently and formatting is skipped when filtered out.
void Write(LogLevel level, const char* module, const char* format, ...);

// Per-session deduplication variant. After formatting, the message body
// (module + formatted text) is checked against a session-scoped set. If the
// same string was already emitted since the last ResetDedup(), the call is
// silently dropped. This keeps debug/trace output readable: each unique
// message appears exactly once until a new game session starts.
void WriteOnce(LogLevel level, const char* module, const char* format, ...);

// Clears the per-session dedup set. Called by PipeWatch when a new game
// process is detected, so the next session re-emits all messages once.
void ResetDedup();

void Shutdown();

// Best-effort flush: pushes any buffered data to disk. Safe to call from
// DllMain(DETACH) before the OS tears down the CRT.
void Flush();

}  // namespace log

namespace diag {

struct Entry {
    std::uint64_t timestampMs = 0;
    std::string category;
    std::string detail;
};

// Lightweight in-process diagnostic ring. It records short lifecycle events for
// status.json without writing a second log file. Thread-safe and best-effort.
void Record(const std::string& category, const std::string& detail);
std::vector<Entry> Snapshot();

}  // namespace diag
}  // namespace ac

// Convenience macros.
#define AC_LOG_TRACE(mod, ...) ::ac::log::Write(::ac::LogLevel::Trace, (mod), __VA_ARGS__)
#define AC_LOG_DEBUG(mod, ...) ::ac::log::Write(::ac::LogLevel::Debug, (mod), __VA_ARGS__)
#define AC_LOG_INFO(mod, ...)  ::ac::log::Write(::ac::LogLevel::Info,  (mod), __VA_ARGS__)
#define AC_LOG_WARN(mod, ...)  ::ac::log::Write(::ac::LogLevel::Warn,  (mod), __VA_ARGS__)
#define AC_LOG_ERROR(mod, ...) ::ac::log::Write(::ac::LogLevel::Error, (mod), __VA_ARGS__)

// Per-session deduplication macros. Each unique (module + formatted message)
// combination is emitted at most once per game session. The dedup set is
// cleared when a new game process starts (see log::ResetDedup()).
#define AC_LOG_TRACE_ONCE(mod, ...) ::ac::log::WriteOnce(::ac::LogLevel::Trace, (mod), __VA_ARGS__)
#define AC_LOG_DEBUG_ONCE(mod, ...) ::ac::log::WriteOnce(::ac::LogLevel::Debug, (mod), __VA_ARGS__)
#define AC_LOG_INFO_ONCE(mod, ...)  ::ac::log::WriteOnce(::ac::LogLevel::Info,  (mod), __VA_ARGS__)
#define AC_LOG_WARN_ONCE(mod, ...)  ::ac::log::WriteOnce(::ac::LogLevel::Warn,  (mod), __VA_ARGS__)
