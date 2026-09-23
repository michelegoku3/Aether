#include "core/AetherCoreState.h"
#include "core/Logger.h"
namespace ac {
TestCoreState g_state;
namespace log {
void Write(LogLevel, const char*, const char*, ...) {}
void SetLevel(LogLevel) {}
LogLevel ParseLevel(const std::string& text, LogLevel fallback) {
    if (text == "debug") return LogLevel::Debug;
    if (text == "info") return LogLevel::Info;
    if (text == "warn") return LogLevel::Warn;
    return fallback;
}
}
namespace diag {
void Record(const std::string&, const std::string&) {}
}
}
