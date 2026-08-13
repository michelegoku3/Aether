#include "pch.h"
#include "hooks/steamclient/OnlineFixHooks.h"

#include <cstring>
#include <fstream>
#include <regex>
#include <sstream>
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

// ---------------------------------------------------------------------------
// SyncLanguageToSpacewar — copies the "language" field from the real game's
// appmanifest ACF to the Spacewar (480) appmanifest ACF.
//
// When a game is masked as 480, Steam reads the language from appmanifest_480.acf.
// If the real game uses Italian but the 480 ACF says "english", the game launches
// in English. This function synchronises them so the game starts in the correct
// language.
//
// ACF format (Valve Data Format):
//   "AppState"
//   {
//       "appid"  "1703340"
//       "UserConfig"
//       {
//           "language"  "italian"
//       }
//   }
// ---------------------------------------------------------------------------
void SyncLanguageToSpacewar(AppId realAppId) {
    if (realAppId == 0 || realAppId == constants::kSpacewarAppId) return;

    const std::string steamPath = g_state.steamInstallPath;
    if (steamPath.empty()) return;

    const std::string realAcf = steamPath + "\\steamapps\\appmanifest_" +
                                std::to_string(realAppId) + ".acf";
    const std::string swAcf   = steamPath + "\\steamapps\\appmanifest_" +
                                std::to_string(constants::kSpacewarAppId) + ".acf";

    // Read the real game's ACF and extract the language field.
    std::ifstream realFile(realAcf);
    if (!realFile.is_open()) {
        AC_LOG_DEBUG(kModule, "SyncLanguage: cannot open %s.", realAcf.c_str());
        return;
    }
    std::string realContent((std::istreambuf_iterator<char>(realFile)),
                             std::istreambuf_iterator<char>());
    realFile.close();

    // Simple regex to find "language" "value" inside the ACF.
    // ACF files are small (< 4 KB) so this is efficient enough.
    std::smatch match;
    std::regex langRegex(R"re("language"\s+"([^"]+)")re", std::regex::icase);
    if (!std::regex_search(realContent, match, langRegex)) {
        AC_LOG_DEBUG(kModule, "SyncLanguage: no language field in appmanifest_%u.acf.", realAppId);
        return;
    }
    const std::string language = match[1].str();
    if (language.empty()) return;

    AC_LOG_INFO(kModule, "SyncLanguage: app %u language='%s'.", realAppId, language.c_str());

    // Read or create the 480 ACF and update/insert the language field.
    std::string swContent;
    {
        std::ifstream swFile(swAcf);
        if (swFile.is_open()) {
            swContent.assign((std::istreambuf_iterator<char>(swFile)),
                              std::istreambuf_iterator<char>());
            swFile.close();
        }
    }

    if (swContent.empty()) {
        // No 480 ACF exists yet — create a minimal one with just the language.
        std::ostringstream oss;
        oss << "\"AppState\"\n{\n"
            << "\t\"appid\"\t\t\"" << constants::kSpacewarAppId << "\"\n"
            << "\t\"UserConfig\"\n\t{\n"
            << "\t\t\"language\"\t\t\"" << language << "\"\n"
            << "\t}\n}\n";
        swContent = oss.str();
    } else if (std::regex_search(swContent, langRegex)) {
        // Replace existing language field.
        swContent = std::regex_replace(swContent,
            std::regex(R"re("language"\s+"[^"]+")re", std::regex::icase),
            "\"language\"\t\t\"" + language + "\"");
    } else {
        // Language field missing — insert it before the last closing brace.
        // Find "UserConfig" section and add language inside it.
        const auto ucPos = swContent.find("\"UserConfig\"");
        if (ucPos != std::string::npos) {
            const auto bracePos = swContent.find('{', ucPos);
            if (bracePos != std::string::npos) {
                swContent.insert(bracePos + 1,
                    "\n\t\t\"language\"\t\t\"" + language + "\"");
            }
        } else {
            // No UserConfig section — add one before the last closing brace.
            const auto lastBrace = swContent.rfind('}');
            if (lastBrace != std::string::npos) {
                std::ostringstream oss;
                oss << "\t\"UserConfig\"\n\t{\n"
                    << "\t\t\"language\"\t\t\"" << language << "\"\n"
                    << "\t}\n";
                swContent.insert(lastBrace, oss.str());
            }
        }
    }

    // Write the updated 480 ACF.
    std::ofstream outFile(swAcf, std::ios::trunc);
    if (outFile.is_open()) {
        outFile << swContent;
        outFile.close();
        AC_LOG_INFO(kModule, "SyncLanguage: wrote language='%s' to appmanifest_480.acf.",
                    language.c_str());
    } else {
        AC_LOG_WARN(kModule, "SyncLanguage: cannot write %s.", swAcf.c_str());
    }
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
            // Synchronise the game's language to the 480 ACF so the game
            // starts in the correct language instead of defaulting to English.
            SyncLanguageToSpacewar(realApp);
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
