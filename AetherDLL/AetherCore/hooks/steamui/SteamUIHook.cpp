#include "pch.h"
#include "hooks/steamui/SteamUIHook.h"

#include <cstring>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "hooks/onlinefix/CreateProcessHooks.h"
#include "hooks/steamclient/DecryptionKeyHook.h"
#include "hooks/steamclient/DepotHooks.h"
#include "core/HookManager.h"
#include "hooks/ipc/IPCBus.h"
#include "hooks/steamclient/LicenseHooks.h"
#include "core/Logger.h"
#include "hooks/steamclient/OnlineFixHooks.h"
#include "hooks/steamclient/OwnershipHooks.h"
#include "hooks/wire/PacketRouter.h"
#include "utils/GameNameResolver.h"
#include "utils/PatternEngine.h"
#include "hooks/ipc/SteamCapture.h"
#include "diagnostics/StatusWriter.h"

namespace ac::hooks {
namespace {

constexpr const char* kModule = "SteamUI";

// steamui.dll!LoadModuleWithPath loads the live steamclient64.dll; we redirect
// it to our diverted, hookable copy.
using LoadModuleWithPath_t = HMODULE (*)(const char*, bool);
LoadModuleWithPath_t o_LoadModuleWithPath = nullptr;

HMODULE h_LoadModuleWithPath(const char* path, bool flags) {
    if (path && std::strstr(path, "steamclient64.dll")) {
        AC_LOG_INFO_ONCE(kModule, "Redirecting steamclient64.dll load to acoverlay.dll.");
        return g_state.diversionModule;
    }
    return o_LoadModuleWithPath(path, flags);
}

// Blocks until steamui.dll is present (or shutdown). Returns the handle or null.
HMODULE WaitForSteamUi() {
    if (HMODULE existing = GetModuleHandleA("steamui.dll")) return existing;
    AC_LOG_INFO(kModule, "Waiting for steamui.dll.");
    while (!g_state.shuttingDown.load()) {
        if (HMODULE h = GetModuleHandleA("steamui.dll")) return h;
        Sleep(constants::kSteamUiPollIntervalMs);
    }
    return nullptr;
}

}  // namespace

void InstallAllHooks() {
    HMODULE steamui = WaitForSteamUi();
    if (!steamui) {
        AC_LOG_WARN(kModule, "steamui.dll never appeared; aborting hook install.");
        return;
    }
    g_state.steamuiModule = steamui;
    AC_LOG_INFO(kModule, "steamui.dll found (0x%p).", steamui);

    // The redirect hook is mandatory; without it our diverted copy is never used.
    if (void* addr = pattern::ResolveAddress("LoadModuleWithPath", "steamui", steamui)) {
        g_state.hookManager.RegisterHook("LoadModuleWithPath", addr,
                                   reinterpret_cast<void**>(&o_LoadModuleWithPath),
                                   reinterpret_cast<void*>(h_LoadModuleWithPath));
    } else {
        g_state.hookManager.RecordMissed("LoadModuleWithPath");
        AC_LOG_WARN(kModule, "LoadModuleWithPath unresolved; steamclient hooks may not apply.");
    }

    // Register every steamclient hook, then enable them all atomically so Steam
    // never observes a half-installed state.

    // kernel32.dll hooks (pre-entry payload injection): install before any game
    // can be launched so the first SpawnProcess → CreateProcessW chain is covered.
    RegisterCreateProcessHooks();

    RegisterOwnershipHooks(g_state.diversionModule);
    RegisterDepotHooks(g_state.diversionModule);
    RegisterDecryptionKeyHook(g_state.diversionModule);
    RegisterOnlineFixHooks(g_state.diversionModule);

    // IPC layer: capture helpers must resolve before the bus arms, and the bus
    // registers its command handlers internally.
    capture::Init(g_state.diversionModule);
    RegisterIpcBus(g_state.diversionModule);

    // Localized titles for presence (game_extra_info / PersonaState game_name).
    // Arms a one-shot capture on GetAppDataFromAppInfo; soft-fails if missing.
    gamename::Init(g_state.diversionModule);

    // Wire layer: outgoing/incoming packet manipulation (presence included).
    RegisterPacketRouter(g_state.diversionModule);

    // License/controller compatibility hooks.
    RegisterLicenseHooks(g_state.diversionModule);

    if (g_state.hookManager.InstallAll()) {
        g_state.hooksInstalled.store(true);
        AC_LOG_INFO(kModule, "All hooks enabled.");
    } else {
        AC_LOG_ERROR(kModule, "Hook enable failed.");
    }

    // Publish the final state for any external monitor regardless of outcome.
    status::Write();
}

}  // namespace ac::hooks
