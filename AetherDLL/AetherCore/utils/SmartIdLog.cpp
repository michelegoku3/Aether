#include "pch.h"
#include "utils/SmartIdLog.h"

#include <algorithm>
#include <mutex>
#include <vector>

namespace ac::logutil {
namespace {

// Private logger-service registry. It only tracks SmartIdLog instances so a
// session reset can fan out to them; it does not store AetherCore domain state.
// Allowed module-local state per docs/ARCHITECTURE.md.
std::mutex s_registryMutex;
std::vector<SmartIdLog*>& Registry() {
    static std::vector<SmartIdLog*> instance;
    return instance;
}

void RegisterLogger(SmartIdLog* logger) {
    std::lock_guard<std::mutex> lock(s_registryMutex);
    Registry().push_back(logger);
}

void UnregisterLogger(SmartIdLog* logger) {
    std::lock_guard<std::mutex> lock(s_registryMutex);
    auto& reg = Registry();
    reg.erase(std::remove(reg.begin(), reg.end(), logger), reg.end());
}

}  // namespace

void RegisterSmartIdLogInstance(SmartIdLog* logger) {
    RegisterLogger(logger);
}

void UnregisterSmartIdLogInstance(SmartIdLog* logger) {
    UnregisterLogger(logger);
}

void ResetAllIdLogSessions() {
    std::lock_guard<std::mutex> lock(s_registryMutex);
    for (auto* logger : Registry()) {
        if (logger) {
            logger->ResetSession();
        }
    }
}

}  // namespace ac::logutil
