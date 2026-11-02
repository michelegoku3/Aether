#include "pch.h"
#include "inject/SteamclientAttach.h"

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "utils/Strings.h"

namespace ac {
namespace {

constexpr const char* kModule = "Steamclient";

// A handle to another copy of the client would be worse than no handle at all:
// the hooks would run on a module Steam never calls, so every feature would look
// installed and do nothing. Only the client at the Steam root is accepted.
bool IsSteamRootModule(HMODULE module) {
    std::wstring path(32768, L'\0');
    const DWORD length = GetModuleFileNameW(module, path.data(), static_cast<DWORD>(path.size()));
    if (length == 0 || length >= path.size())
        return false;
    return _wcsicmp(path.c_str(), strings::Widen(g_state.steamclientPath).c_str()) == 0;
}

HMODULE FindSteamclient() {
    HMODULE module = GetModuleHandleA("steamclient64.dll");
    if (module && !IsSteamRootModule(module)) {
        AC_LOG_DEBUG_ONCE(kModule, "A steamclient64.dll is mapped from another directory; waiting for Steam's.");
        return nullptr;
    }
    return module;
}

} // namespace

bool AttachSteamclient(std::uint32_t timeoutMs) {
    if (g_state.steamclientModule)
        return true;

    g_state.steamclientPath = g_state.steamInstallPath + "\\steamclient64.dll";
    if (strings::Widen(g_state.steamclientPath).empty()) {
        g_state.steamclientAttach = "path-invalid";
        AC_LOG_ERROR(kModule, "Steam client path is not valid UTF-8; aborting attach.");
        return false;
    }

    const ULONGLONG start = GetTickCount64();
    for (;;) {
        if (HMODULE module = FindSteamclient()) {
            g_state.steamclientModule = module;
            g_state.steamclientAttach = "live-mapped";
            AC_LOG_INFO(kModule, "Steamclient mapped by Steam after %llu ms (handle 0x%p).", GetTickCount64() - start,
                        module);
            return true;
        }
        if (GetTickCount64() - start >= timeoutMs)
            break;
        Sleep(static_cast<DWORD>(constants::kSteamclientAttachPollMs));
    }

    g_state.steamclientModule = nullptr;
    g_state.steamclientAttach = "live-not-mapped";
    AC_LOG_ERROR(kModule, "Steam did not map %s within %u ms; the hook batch stays disabled.",
                 g_state.steamclientPath.c_str(), timeoutMs);
    return false;
}

} // namespace ac
