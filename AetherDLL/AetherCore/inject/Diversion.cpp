#include "pch.h"
#include "inject/Diversion.h"

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"

namespace ac {
namespace {
constexpr const char* kModule = "Diversion";

// Retries an action that may transiently fail while Steam holds a file lock.
template <typename Op>
bool RetryWithBackoff(Op&& op) {
    for (int i = 0; i < constants::kDiversionMaxRetries; ++i) {
        if (op()) return true;
        Sleep(constants::kDiversionRetryDelayMs);
    }
    return false;
}

}  // namespace

bool LoadDiversion() {
    AC_LOG_INFO(kModule, "Starting diversion.");

    g_state.diversionOutcome = "not-attempted";

    g_state.steamclientPath = g_state.steamInstallPath + "\\steamclient64.dll";
    const std::string binDir = g_state.steamInstallPath + "\\bin";
    g_state.diversionPath = binDir + "\\acoverlay.dll";

    if (!CreateDirectoryA(binDir.c_str(), nullptr) && GetLastError() != ERROR_ALREADY_EXISTS) {
        AC_LOG_ERROR(kModule, "Could not create bin directory.");
        g_state.diversionOutcome = "copy-failed";
        return false;
    }

    const bool copied = RetryWithBackoff([&] {
        return CopyFileA(g_state.steamclientPath.c_str(), g_state.diversionPath.c_str(), FALSE) != 0;
    });
    if (!copied) {
        AC_LOG_ERROR(kModule, "Failed to copy steamclient64.dll after retries.");
        g_state.diversionOutcome = "copy-failed";
        return false;
    }
    AC_LOG_INFO(kModule, "Copied steamclient64.dll -> acoverlay.dll.");

    const bool loaded = RetryWithBackoff([&] {
        g_state.diversionModule = LoadLibraryA(g_state.diversionPath.c_str());
        return g_state.diversionModule != nullptr;
    });
    if (!loaded) {
        AC_LOG_ERROR(kModule, "Failed to load acoverlay.dll after retries.");
        g_state.diversionOutcome = "load-failed";
        return false;
    }

    g_state.diversionOutcome = "loaded";
    AC_LOG_INFO(kModule, "acoverlay.dll loaded (handle 0x%p).", g_state.diversionModule);
    return true;
}

}  // namespace ac
