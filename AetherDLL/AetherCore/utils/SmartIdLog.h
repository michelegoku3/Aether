#pragma once

#include <mutex>
#include <string>
#include <unordered_set>

#include "core/Logger.h"
#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Per-Session Deduplicating Logger for AppIds.
//
// Behaviour:
//   * First occurrence of an AppId is logged immediately.
//   * All subsequent occurrences within the same game session are silently
//     suppressed — no periodic flushing, no time-based windows.
//   * ResetSession() clears the seen-set so the next game session re-emits
//     every AppId once. Called automatically when a new game process starts.
//
// Rationale (replaces the previous time-windowed debounce):
//   The old 10-second debounce still produced periodic bursts of identical
//   "Unlocked AppIds" / "RequiresLegacyCDKey suppressed" lines that cluttered
//   debug and trace logs without adding diagnostic value. A game session is a
//   natural dedup boundary: ownership and license outcomes are stable within a
//   session, so logging them once is sufficient.
// ---------------------------------------------------------------------------
namespace ac::logutil {

class SmartIdLog;

void RegisterSmartIdLogInstance(SmartIdLog* logger);
void UnregisterSmartIdLogInstance(SmartIdLog* logger);
void ResetAllIdLogSessions();

class SmartIdLog {
public:
    SmartIdLog(const char* module, const char* label)
        : module_(module ? module : "Log"),
          label_(label ? label : "AppIds") {
        RegisterSmartIdLogInstance(this);
    }

    ~SmartIdLog() {
        UnregisterSmartIdLogInstance(this);
    }

    // Non-copyable/non-movable due to self-registration pointer
    SmartIdLog(const SmartIdLog&) = delete;
    SmartIdLog& operator=(const SmartIdLog&) = delete;

    void Record(steam::AppId id) {
        std::lock_guard<std::mutex> lock(mutex_);
        if (!seen_.insert(id).second) {
            return;  // Already logged this session — suppress silently.
        }
        AC_LOG_INFO(module_.c_str(), "%s: AppId %u.", label_.c_str(), id);
    }

    void ResetSession() {
        std::lock_guard<std::mutex> lock(mutex_);
        seen_.clear();
    }

private:
    std::mutex mutex_;
    std::unordered_set<steam::AppId> seen_;
    std::string module_;
    std::string label_;
};

}  // namespace ac::logutil
