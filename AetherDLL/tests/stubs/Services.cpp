#include "core/AetherCoreState.h"
#include "core/Logger.h"
namespace ac {
TestCoreState g_state;
namespace log {
void Write(LogLevel, const char*, const char*, ...) {}
void WriteOnce(LogLevel, const char*, const char*, ...) {}
void SetLevel(LogLevel) {}
LogLevel ParseLevel(const std::string& text, LogLevel fallback) {
    if (text == "debug")
        return LogLevel::Debug;
    if (text == "info")
        return LogLevel::Info;
    if (text == "warn")
        return LogLevel::Warn;
    return fallback;
}
} // namespace log
namespace diag {
// Recording fake (not the 64-entry ring): tests assert on emitted
// categories, e.g. config_deprecated_manifest_fetch. Reset per test
// binary run is unnecessary: ctest runs one case per process.
std::vector<Entry> s_records;
void Record(const std::string& category, const std::string& detail) {
    Entry e;
    e.category = category;
    e.detail = detail;
    s_records.push_back(std::move(e));
}
std::vector<Entry> Snapshot() { return s_records; }
} // namespace diag
} // namespace ac
