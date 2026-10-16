#include "pch.h"
#include "inject/Diversion.h"

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "core/Settings.h"
#include "utils/Strings.h"

namespace ac {
namespace {
constexpr const char* kModule = "Diversion";

template <typename Op> bool RetryWithBackoff(Op&& op) {
    for (int i = 0; i < constants::kDiversionMaxRetries; ++i) {
        if (op())
            return true;
        Sleep(constants::kDiversionRetryDelayMs);
    }
    return false;
}

void SetOutcome(const char* outcome) {
    std::lock_guard lock(g_state.statusMetadataMutex);
    g_state.diversionOutcome = outcome;
}

// GetModuleHandle by basename is insufficient: another file of that name may
// have been mapped from elsewhere. Auto mode only attaches to Steam's own DLL.
bool LiveSteamclientMapped() {
    HMODULE live = GetModuleHandleA("steamclient64.dll");
    if (!live)
        return false;
    // Wide at the boundary (UTF-8 inside): *A would mangle non-ASCII Steam
    // installs via the system codepage. Compare in wide (case-insensitive).
    std::wstring wide(32768, L'\0');
    const DWORD length = GetModuleFileNameW(live, wide.data(), static_cast<DWORD>(wide.size()));
    if (length == 0 || length >= wide.size() ||
        _wcsicmp(wide.c_str(), ac::strings::Widen(g_state.steamclientPath).c_str()) != 0) {
        AC_LOG_WARN(kModule, "Mapped steamclient64.dll is not at the Steam root; refusing auto-live.");
        return false;
    }
    return true;
}

bool AttachLive(const char* outcome) {
    HMODULE live = LoadLibraryW(ac::strings::Widen(g_state.steamclientPath).c_str());
    if (!live) {
        const DWORD error = GetLastError();
        SetOutcome("live-load-failed");
        AC_LOG_ERROR(kModule, "Could not load live steamclient64.dll (error %lu).", error);
        return false;
    }
    g_state.diversionModule = live;
    g_state.diversionPath = g_state.steamclientPath;
    g_state.diversionUsesLive.store(true, std::memory_order_release);
    SetOutcome(outcome);
    AC_LOG_INFO(kModule, "Hook target: live steamclient64.dll (handle 0x%p, %s).", live, outcome);
    return true;
}
} // namespace

bool LoadDiversion() {
    g_state.steamclientPath = g_state.steamInstallPath + "\\steamclient64.dll";
    const auto settings = Settings::Snapshot();
    SetOutcome("not-attempted");

    if (settings->diversionMode == DiversionMode::Live) {
        return AttachLive("live-loaded");
    }
    // Auto preserves the copy path unless the real module was already loaded
    // before we could arm steamui's LoadModuleWithPath redirect.
    if (settings->diversionMode == DiversionMode::Auto && LiveSteamclientMapped()) {
        return AttachLive("auto-live-preloaded");
    }

    const std::string binDir = g_state.steamInstallPath + "\\bin";
    g_state.diversionPath = binDir + "\\acoverlay.dll";
    const std::wstring wBin = ac::strings::Widen(binDir);
    const std::wstring wClient = ac::strings::Widen(g_state.steamclientPath);
    const std::wstring wDiversion = ac::strings::Widen(g_state.diversionPath);
    if (wBin.empty() || wClient.empty() || wDiversion.empty()) {
        AC_LOG_ERROR(kModule, "Steam paths are not valid UTF-8; aborting diversion.");
        SetOutcome("copy-failed");
        return settings->diversionMode == DiversionMode::Auto && AttachLive("auto-live-copy-failed");
    }
    if (!CreateDirectoryW(wBin.c_str(), nullptr) && GetLastError() != ERROR_ALREADY_EXISTS) {
        AC_LOG_ERROR(kModule, "Could not create bin directory.");
        SetOutcome("copy-failed");
        return settings->diversionMode == DiversionMode::Auto && AttachLive("auto-live-copy-failed");
    }
    if (!RetryWithBackoff([&] { return CopyFileW(wClient.c_str(), wDiversion.c_str(), FALSE) != 0; })) {
        AC_LOG_ERROR(kModule, "Failed to copy steamclient64.dll after retries.");
        SetOutcome("copy-failed");
        return settings->diversionMode == DiversionMode::Auto && AttachLive("auto-live-copy-failed");
    }
    if (!RetryWithBackoff([&] {
            g_state.diversionModule = LoadLibraryW(wDiversion.c_str());
            return g_state.diversionModule != nullptr;
        })) {
        AC_LOG_ERROR(kModule, "Failed to load acoverlay.dll after retries.");
        SetOutcome("copy-load-failed");
        return settings->diversionMode == DiversionMode::Auto && AttachLive("auto-live-copy-load-failed");
    }
    SetOutcome(settings->diversionMode == DiversionMode::Auto ? "auto-copy-prepared" : "copy-loaded");
    AC_LOG_INFO(kModule, "Hook target prepared: acoverlay.dll (handle 0x%p).", g_state.diversionModule);
    return true;
}

void SelectHookTargetBeforeRedirect() {
    if (Settings::Snapshot()->diversionMode != DiversionMode::Auto ||
        g_state.diversionUsesLive.load(std::memory_order_acquire))
        return;

    // Called after pattern fetching but BEFORE redirect installation. If Steam
    // loaded the real client in the meantime, the copy would receive hooks yet
    // not be the client Steam actually uses. Attach live instead; do not arm a
    // late redirect that would split a single session between two instances.
    if (LiveSteamclientMapped()) {
        if (!AttachLive("auto-live-late")) {
            AC_LOG_ERROR(kModule, "Auto-live attach failed; retaining the copy target.");
            SetOutcome("auto-copy-live-attach-failed");
        }
    } else {
        SetOutcome("auto-copy");
    }
}

} // namespace ac
