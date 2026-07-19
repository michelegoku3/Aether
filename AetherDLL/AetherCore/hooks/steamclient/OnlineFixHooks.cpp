#include "pch.h"
#include "hooks/steamclient/OnlineFixHooks.h"

#include <cstring>
#include <string>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/HookManager.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "core/SteamTypes.h"

namespace ac::hooks {
namespace {

constexpr const char* kModule = "OnlineFix";
using namespace ac::steam;

// pGameID points at a uint64 GameID whose low 24 bits hold the AppId.
using SpawnProcess_t = bool (*)(void*, const char*, const char*, const char*, std::uint64_t*,
                               const void*, std::uint32_t, std::int32_t);
using GetAppIDForCurrentPipe_t = AppId (*)(void*);

SpawnProcess_t o_SpawnProcess = nullptr;
GetAppIDForCurrentPipe_t o_GetAppIDForCurrentPipe = nullptr;

// Checks whether cmdLine contains "-onlinefix" as a whole argument
// (space-delimited), not as a substring. strstr() would match "-onlinefix2"
// or "--onlinefix" which is incorrect — only the exact argument triggers
// the Spacewar/480 masking.
static bool HasOnlineFixFlag(const char* cmdLine) {
    if (!cmdLine) return false;
    std::string cl(cmdLine);
    std::size_t pos = 0;
    while (pos < cl.size()) {
        while (pos < cl.size() && (cl[pos] == ' ' || cl[pos] == '\t')) ++pos;
        if (pos >= cl.size()) break;
        std::size_t end = cl.find(' ', pos);
        if (end == std::string::npos) end = cl.size();
        if (cl.substr(pos, end - pos) == constants::kOnlineFixFlag) return true;
        pos = end;
    }
    return false;
}

bool h_SpawnProcess(void* user, const char* exe, const char* cmdLine, const char* workDir,
                    std::uint64_t* gameId, const void* blob, std::uint32_t blobSize,
                    std::int32_t launchOption) {
    if (gameId) {
        AppId realApp = static_cast<AppId>(*gameId & constants::kGameIdAppIdMask);
        bool isOnlineFix = HasOnlineFixFlag(cmdLine);

        if (isOnlineFix && luadata::HasDepot(realApp)) {
            g_state.onlineFixRealAppId.store(realApp);
            *gameId = (*gameId & ~constants::kGameIdAppIdMask) | constants::kSpacewarAppId;
            AC_LOG_INFO(kModule, "Masked AppId %u as Spacewar (%u) for OnlineFix.",
                        realApp, constants::kSpacewarAppId);
        } else {
            g_state.onlineFixRealAppId.store(0);
        }
    }
    return o_SpawnProcess(user, exe, cmdLine, workDir, gameId, blob, blobSize, launchOption);
}

AppId h_GetAppIDForCurrentPipe(void* engine) {
    void* prev = nullptr;
    if (g_state.steamEngine.compare_exchange_strong(prev, engine)) {
        AC_LOG_INFO(kModule, "Captured steamEngine pointer 0x%p.", engine);
    }
    // Return Steam's original value unchanged. OnlineFix masks the process as
    // Spacewar/480 for multiplayer routing. The real app identity for DLC and
    // overlay queries comes from SteamOverlayGameId, patched by
    // h_BuildSpawnEnvBlock below.
    //
    // MUST NOT translate 480 → real here. That was the pre-9aa4a76 "Meccha"
    // regression path: it fixed DLC as a side effect but leaked real app
    // identity into multiplayer routing, and the reverse (passthrough) made
    // friends presence look like Spacewar. Friends/UI presence is handled by
    // the wire pipeline (GamesPlayed extra_info + PersonaInject), never by
    // GetAppID. See docs/03-presence-identity-plan.md.
    return o_GetAppIDForCurrentPipe(engine);
}

// -----------------------------------------------------------------------
// BuildSpawnEnvBlock — patches the overlay CGameID from 480 to the real
// app id so internal Steam queries (DLC enumeration, depot metadata,
// overlay identity) see the real app while the process-tracking CGameID
// stays on 480 for multiplayer routing.
//
// This is the mechanism LumaCore uses to make both DLC and online
// multiplayer work simultaneously. Without it, one breaks the other.
// -----------------------------------------------------------------------
using BuildSpawnEnvBlock_t = std::int64_t (*)(
    void*, std::uint64_t*, void*, void*,
    std::uint64_t*, void*, std::int32_t,
    void*, void*, std::uint32_t, char);

BuildSpawnEnvBlock_t o_BuildSpawnEnvBlock = nullptr;

std::int64_t h_BuildSpawnEnvBlock(
    void* pThis, std::uint64_t* pCGameID, void* a3, void* env,
    std::uint64_t* pOverlayCGameID, void* a6, std::int32_t a7,
    void* a8, void* a9, std::uint32_t a10, char a11)
{
    AppId realAppId = g_state.onlineFixRealAppId.load();

    if (realAppId && pOverlayCGameID) {
        AppId overlayAppId = static_cast<AppId>(
            *pOverlayCGameID & constants::kGameIdAppIdMask);
        if (overlayAppId == constants::kSpacewarAppId) {
            *pOverlayCGameID =
                (*pOverlayCGameID & ~static_cast<std::uint64_t>(constants::kGameIdAppIdMask))
                | static_cast<std::uint64_t>(realAppId);
            AC_LOG_INFO(kModule, "BuildSpawnEnvBlock: overlay %u -> %u.",
                        overlayAppId, realAppId);
        }
    }

    return o_BuildSpawnEnvBlock(pThis, pCGameID, a3, env, pOverlayCGameID,
                                a6, a7, a8, a9, a10, a11);
}

}  // namespace (anonymous)

// -----------------------------------------------------------------------
// Public API — defined in namespace ac::hooks (NOT anonymous) so the
// linker can resolve cross-TU calls from SteamCapture / SteamUIHook.
// -----------------------------------------------------------------------

steam::AppId CallOriginalGetAppIdForCurrentPipe() {
    void* engine = g_state.steamEngine.load();
    if (!o_GetAppIDForCurrentPipe || !engine) return 0;
    return o_GetAppIDForCurrentPipe(engine);
}

void RegisterOnlineFixHooks(HMODULE diversion) {
    if (!diversion) {
        AC_LOG_ERROR(kModule, "Diversion module not loaded.");
        return;
    }
    AC_LOG_INFO(kModule, "Registering OnlineFix hooks.");
    g_state.hookManager.TryHook("SpawnProcess", "steamclient", diversion,
                          o_SpawnProcess, h_SpawnProcess);
    g_state.hookManager.TryHook("GetAppIDForCurrentPipe", "steamclient", diversion,
                          o_GetAppIDForCurrentPipe, h_GetAppIDForCurrentPipe);
    g_state.hookManager.TryHook("BuildSpawnEnvBlock", "steamclient", diversion,
                          o_BuildSpawnEnvBlock, h_BuildSpawnEnvBlock);
}

}  // namespace ac::hooks
