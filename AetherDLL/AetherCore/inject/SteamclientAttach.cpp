#include "pch.h"
#include "inject/SteamclientAttach.h"

#include "core/AetherCoreState.h"
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

} // namespace

bool AttachSteamclient() {
    g_state.steamclientPath = g_state.steamInstallPath + "\\steamclient64.dll";
    const std::wstring path = strings::Widen(g_state.steamclientPath);
    if (path.empty()) {
        g_state.steamclientAttach = "path-invalid";
        AC_LOG_ERROR(kModule, "Steam client path is not valid UTF-8; aborting attach.");
        return false;
    }

    // Steam normally maps its client during startup. Reuse that mapping when it
    // is already there so hooks and Steam share one module; load it only when
    // Steam has not mapped it yet.
    HMODULE module = GetModuleHandleA("steamclient64.dll");
    if (module && !IsSteamRootModule(module)) {
        AC_LOG_WARN(kModule, "A steamclient64.dll is mapped from another directory; ignoring it.");
        module = nullptr;
    }
    const bool alreadyMapped = module != nullptr;

    if (!module)
        module = LoadLibraryW(path.c_str());

    if (!module || !IsSteamRootModule(module)) {
        if (module && !alreadyMapped)
            FreeLibrary(module);
        g_state.steamclientModule = nullptr;
        g_state.steamclientAttach = "live-load-failed";
        AC_LOG_ERROR(kModule, "Could not attach to %s.", g_state.steamclientPath.c_str());
        return false;
    }

    g_state.steamclientModule = module;
    g_state.steamclientAttach = alreadyMapped ? "live-already-mapped" : "live-loaded";
    AC_LOG_INFO(kModule, "Hook target: live steamclient64.dll (handle 0x%p, %s).", module,
                g_state.steamclientAttach.c_str());
    return true;
}

} // namespace ac
