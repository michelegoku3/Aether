#include "pch.h"
#include "hooks/steamui/SteamUIHook.h"

#include <algorithm>
#include <cstring>

#include "core/AetherCoreState.h"
#include "core/HookBootstrap.h"
#include "core/HookManager.h"
#include "core/Logger.h"
#include "diagnostics/StatusWriter.h"
#include "utils/PatternEngine.h"

namespace ac::hooks::steamui {
namespace {

constexpr const char* kModule = "SteamUI";

// In copy mode, steamui.dll!LoadModuleWithPath must receive the copy. In
// live mode it must run unmodified. Armed by core/HookBootstrap as soon as
// the pattern table is ready, not after Lua/IPC/network initialization.
using LoadModuleWithPath_t = HMODULE (*)(const char*, bool);
LoadModuleWithPath_t o_LoadModuleWithPath = nullptr;

HMODULE h_LoadModuleWithPath(const char* path, bool flags) {
    if (path && !g_state.diversionUsesLive.load(std::memory_order_acquire)) {
        const char* backslash = std::strrchr(path, '\\');
        const char* slash = std::strrchr(path, '/');
        const char* name = backslash && (!slash || backslash > slash) ? backslash + 1
                         : slash ? slash + 1 : path;
        if (_stricmp(name, "steamclient64.dll") == 0 && g_state.diversionModule) {
            g_state.steamUiRedirectUsed.store(true, std::memory_order_release);
            AC_LOG_INFO_ONCE(kModule, "Redirecting steamclient64.dll to acoverlay.dll.");
            status::Write();
            return g_state.diversionModule;
        }
    }
    return o_LoadModuleWithPath(path, flags);
}

}  // namespace

bool InstallSteamUiRedirect() {
    if (g_state.diversionUsesLive.load(std::memory_order_acquire)) {
        AC_LOG_INFO_ONCE(kModule, "Live mode: SteamUI redirect not needed "
                                  "(hooking the live steamclient directly).");
        return true;
    }
    std::lock_guard<std::mutex> batchLock(bootstrap::BatchMutex());
    if (g_state.steamUiRedirectInstalled.load(std::memory_order_acquire)) return true;
    HMODULE steamui = GetModuleHandleA("steamui.dll");
    if (!steamui) {
        AC_LOG_DEBUG(kModule, "SteamUI redirect deferred: steamui.dll not mapped yet.");
        return false;
    }
    g_state.steamuiModule = steamui;

    void* addr = pattern::ResolveAddress("LoadModuleWithPath", "steamui", steamui);
    if (!addr) {
        g_state.hookManager.RecordMissed("LoadModuleWithPath", MissReason::PatternUnresolved);
        status::Write();
        AC_LOG_DEBUG(kModule, "SteamUI redirect deferred: LoadModuleWithPath pattern unresolved.");
        return false;
    }
    g_state.hookManager.RegisterHook("LoadModuleWithPath", addr,
                                    reinterpret_cast<void**>(&o_LoadModuleWithPath),
                                    reinterpret_cast<void*>(h_LoadModuleWithPath));
    const bool enabled = g_state.hookManager.InstallAll();
    const auto hooks = g_state.hookManager.Snapshot();
    const bool created = std::find(hooks.installed.begin(), hooks.installed.end(),
                                   "LoadModuleWithPath") != hooks.installed.end();
    if (enabled && created) {
        g_state.steamUiRedirectInstalled.store(true, std::memory_order_release);
        AC_LOG_INFO(kModule, "SteamUI redirect installed.");
    } else {
        AC_LOG_ERROR(kModule, "SteamUI redirect not enabled; retrying (enabled=%d created=%d).",
                     enabled ? 1 : 0, created ? 1 : 0);
    }
    status::Write();
    return enabled && created;
}

}  // namespace ac::hooks::steamui
