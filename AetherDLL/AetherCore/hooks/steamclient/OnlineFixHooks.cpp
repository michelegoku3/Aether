#include "pch.h"
#include "hooks/steamclient/OnlineFixHooks.h"

#include <algorithm>
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
#include "hooks/ipc/SteamCapture.h"

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

// Checks whether cmdLine contains 'flag' as a whole argument
// (space-delimited), not as a substring. strstr() would match "-onlinefix2"
// or "--onlinefix" which is incorrect — only the exact argument triggers
// the special session modes.
static bool HasFlagArg(const char* cmdLine, const char* flag) {
    if (!cmdLine) return false;
    std::string cl(cmdLine);
    std::size_t pos = 0;
    while (pos < cl.size()) {
        while (pos < cl.size() && (cl[pos] == ' ' || cl[pos] == '\t')) ++pos;
        if (pos >= cl.size()) break;
        std::size_t end = cl.find_first_of(" \t", pos);  // whitespace-consistent with StripAetherFlagArgs
        if (end == std::string::npos) end = cl.size();
        if (cl.substr(pos, end - pos) == flag) return true;
        pos = end;
    }
    return false;
}

static bool HasOnlineFixFlag(const char* cmdLine) {
    return HasFlagArg(cmdLine, constants::kOnlineFixFlag);
}

static bool HasShowOnlineFlag(const char* cmdLine) {
    return HasFlagArg(cmdLine, constants::kShowOnlineFlag);
}

static bool FileExists(const std::string& path) {
    std::ifstream f(path);
    return f.good();
}

static std::string DirectoryOf(const char* path) {
    if (!path || !*path) return {};
    std::string s(path);
    if (!s.empty() && s.front() == '"') {
        s.erase(s.begin());
        if (!s.empty() && s.back() == '"') s.pop_back();
    }
    const auto slash = s.find_last_of("\\/");
    return (slash == std::string::npos) ? std::string{} : s.substr(0, slash);
}

// UCO2 (union-crax.ini) or OFME/SteamFix next to the exe: the process will
// spoof Spacewar. Show Online must not start — same default as Desk None.
static bool HasSpacewarSpoofOnDisk(const char* exe, const char* workDir) {
    std::string dirs[3];
    int n = 0;
    const std::string exeDir = DirectoryOf(exe);
    if (!exeDir.empty()) {
        dirs[n++] = exeDir;
        const std::string parent = DirectoryOf(exeDir.c_str());
        if (!parent.empty()) dirs[n++] = parent;
    }
    if (workDir && *workDir) dirs[n++] = workDir;

    static constexpr const char* kMarkers[] = {
        "union-crax.ini",
        "OnlineFix64.dll", "OnlineFix.dll", "OnlineFix.ini", "OnlineFix.json",
        "SteamFix64.dll", "SteamFix.dll", "SteamFix.ini",
    };
    for (int i = 0; i < n; ++i) {
        for (const char* marker : kMarkers) {
            if (FileExists(dirs[i] + "\\" + marker) || FileExists(dirs[i] + "/" + marker)) {
                return true;
            }
        }
    }
    return false;
}

// Centralised per-app launch policy (docs/05 §12): is the launch a normal
// one, a -showonline presence session, or a -onlinefix masked session?
// The TOML arrays in [presence] are the source of truth; the legacy argv
// tokens keep working for unmigrated configs but are always stripped from
// the child command line below.
enum class LaunchMode : std::uint8_t { None, ShowOnline, OnlineFix };

static bool InAppList(const std::vector<std::uint32_t>& list, AppId app) {
    return std::find(list.begin(), list.end(), static_cast<std::uint32_t>(app)) != list.end();
}

// Precedence (documented): exclude > onlinefix > showonline > default_mode.
// exclude is a HARD opt-out: it beats even a forgotten legacy argv token.
static LaunchMode ResolveLaunchMode(AppId app, bool onlineFixToken, bool showOnlineToken,
                                    const char** sourceOut) {
    const auto& s = g_state.settings;
    *sourceOut = "none";
    if (InAppList(s.presenceExcludeApps, app)) {
        *sourceOut = "exclude_apps (hard opt-out)";
        return LaunchMode::None;
    }
    if (InAppList(s.presenceOnlineFixApps, app)) {
        *sourceOut = "onlinefix_apps";
        return LaunchMode::OnlineFix;
    }
    if (InAppList(s.presenceShowOnlineApps, app)) {
        *sourceOut = "showonline_apps";
        return LaunchMode::ShowOnline;
    }
    // Global custom presence name (docs/05 §13): when set, EVERY app without
    // an explicit entry behaves like a -showonline session — presence-only,
    // never the OnlineFix mask. The name itself already overrides the display
    // in BOTH pipelines via DisplayName() (GamesPlayedModule), so -onlinefix
    // games also show the custom name while keeping their mask.
    if (!s.presenceCustomGameName.empty()) {
        *sourceOut = "custom_game_name (global presence override)";
        return LaunchMode::ShowOnline;
    }
    if (onlineFixToken) {
        *sourceOut = "legacy -onlinefix argv token";
        return LaunchMode::OnlineFix;
    }
    if (showOnlineToken) {
        *sourceOut = "legacy -showonline argv token";
        return LaunchMode::ShowOnline;
    }
    if (s.presenceDefaultShowOnline) {
        *sourceOut = "default_mode=showonline";
        return LaunchMode::ShowOnline;
    }
    return LaunchMode::None;
}

// Returns cmdLine minus every Aether control token (-onlinefix / -showonline).
// Same whitespace-tokenisation as HasFlagArg; outStripped tells whether at
// least one token was removed. Aether consumes those flags here, in
// SpawnProcess — the child process must NEVER see them in argv: Steam itself
// ignores unknown launch arguments, but some games hard-crash on them.
// MEASURED, 2026-08-24 log: "Selene ~Apoptosis~" exits 3-4 s after every
// launch with -showonline left in the command line (three runs in a row,
// 4.6 s / 4.2 s lifetimes) and runs ~28 s on the flag-less launch. Gamblers
// Table and Stanley Parable tolerate it; strict argv parsers do not.
static std::string StripAetherFlagArgs(const char* cmdLine, bool* outStripped) {
    std::string out;
    if (outStripped) *outStripped = false;
    if (!cmdLine) return out;
    const std::string cl(cmdLine);
    out.reserve(cl.size());
    std::size_t pos = 0;
    while (pos < cl.size()) {
        while (pos < cl.size() && (cl[pos] == ' ' || cl[pos] == '\t')) ++pos;
        if (pos >= cl.size()) break;
        std::size_t end = cl.find_first_of(" \t", pos);
        if (end == std::string::npos) end = cl.size();
        const std::string tok = cl.substr(pos, end - pos);
        pos = end;
        if (tok == constants::kOnlineFixFlag || tok == constants::kShowOnlineFlag) {
            if (outStripped) *outStripped = true;
            continue;
        }
        if (!out.empty()) out.push_back(' ');
        out += tok;
    }
    return out;
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
    std::string childCmdStorage;
    const char* childCmd = cmdLine;
    if (gameId) {
        // Refresh config before deciding: AetherDesk edits the [presence]
        // arrays while Steam is running, and a cold launch may reach this
        // hook before the next GamesPlayed frame would reload (mtime, cheap).
        Settings::ReloadIfModified(g_state.configPath);

        AppId realApp = static_cast<AppId>(*gameId & constants::kGameIdAppIdMask);
        if (realApp != 0 && realApp != constants::kSpacewarAppId) {
            g_state.lastSpawnedAppId.store(realApp);
        }

        // Centralised resolution (docs/05 §12): TOML arrays are the source of
        // truth; argv tokens (-onlinefix / -showonline) are LEGACY hints that
        // keep working for unmigrated configs and are ALWAYS stripped from
        // the child command line below. exclude_apps wins over tokens too.
        const bool hasOfToken = HasOnlineFixFlag(cmdLine);
        const bool hasSoToken = HasShowOnlineFlag(cmdLine);
        const char* modeSource = nullptr;
        LaunchMode mode = ResolveLaunchMode(realApp, hasOfToken, hasSoToken, &modeSource);
        const bool spoofOnDisk = HasSpacewarSpoofOnDisk(exe, workDir);
        g_state.spacewarSpoofExpected.store(spoofOnDisk);
        if (mode == LaunchMode::ShowOnline && spoofOnDisk) {
            mode = LaunchMode::None;
            modeSource = "UCO2/OFME on disk; skip showonline";
        }
        // Verdict line for EVERY launch (including mode None / depot misses,
        // which the branches below leave silent): with the build stamp this
        // tells you exactly what the DLL decided and why — marker read vs
        // argv token vs default — instead of guessing after the fact.
        AC_LOG_INFO(kModule,
                    "Presence resolve: app %u -> %s (source: %s; argv onlinefix=%d showonline=%d; depot=%d).",
                    realApp,
                    mode == LaunchMode::OnlineFix ? "onlinefix"
                        : mode == LaunchMode::ShowOnline ? "showonline" : "none",
                    modeSource ? modeSource : "unknown",
                    hasOfToken ? 1 : 0, hasSoToken ? 1 : 0,
                    luadata::HasDepot(realApp) ? 1 : 0);

        // Hand the child process a clean argv whenever an Aether token was
        // present (see StripAetherFlagArgs). Never null; empty string only
        // when the token was the whole command line.
        if (hasOfToken || hasSoToken) {
            bool stripped = false;
            childCmdStorage = StripAetherFlagArgs(cmdLine, &stripped);
            if (stripped) {
                childCmd = childCmdStorage.c_str();
                AC_LOG_INFO(kModule,
                            "Stripped Aether launch flags from child cmdline "
                            "(app %u, was '%s').",
                            realApp, cmdLine ? cmdLine : "");
            }
        }

        if (mode == LaunchMode::OnlineFix && luadata::HasDepot(realApp)) {
            // OnlineFix: full 480 process mask — a strict superset of what
            // -showonline needs (server presence + friend notification), and
            // the mask is what real multiplayer through a crack requires.
            g_state.onlineFixRealAppId.store(realApp);
            g_state.showOnlineAppId.store(0);
            *gameId = (*gameId & ~constants::kGameIdAppIdMask) | constants::kSpacewarAppId;
            AC_LOG_INFO(kModule,
                        "Masked AppId %u as Spacewar (%u) for OnlineFix (source: %s).",
                        realApp, constants::kSpacewarAppId, modeSource);
            // Synchronise the game's language to the 480 ACF so the game
            // starts in the correct language instead of defaulting to English.
            SyncLanguageToSpacewar(realApp);
        } else if (mode == LaunchMode::ShowOnline && luadata::HasDepot(realApp)) {
            // ShowOnline session: NO process mask. The game stays registered
            // under its real appid, so achievements, DLC, cloud, overlay,
            // screenshots and the community hub behave exactly like a
            // flag-less launch. Only the outgoing presence frames are
            // rewritten to Spacewar/480 on the wire (GamesPlayedModule), so
            // friends still get the "now playing" broadcast.
            g_state.onlineFixRealAppId.store(0);
            g_state.showOnlineAppId.store(realApp);
            AC_LOG_INFO(kModule,
                        "ShowOnline session for app %u: process NOT masked; "
                        "wire-level presence rewrite only (source: %s).",
                        realApp, modeSource);
        } else {
            g_state.onlineFixRealAppId.store(0);
            g_state.showOnlineAppId.store(0);
        }
    }
    return o_SpawnProcess(user, exe, childCmd, workDir, gameId, blob, blobSize, launchOption);
}

AppId h_GetAppIDForCurrentPipe(void* engine) {
    void* prev = nullptr;
    if (g_state.steamEngine.compare_exchange_strong(prev, engine)) {
        AC_LOG_INFO(kModule, "Captured steamEngine pointer 0x%p.", engine);
    }

    AppId appId = o_GetAppIDForCurrentPipe(engine);

    // Scoped OnlineFix stats override (see capture::EnterStatsScope).
    //
    // OnlineFix masks the process as Spacewar/480 for multiplayer routing. The
    // real app identity for DLC and overlay queries comes from
    // SteamOverlayGameId, patched by h_BuildSpawnEnvBlock below, and the
    // friends/UI presence is handled by the wire pipeline (GamesPlayed
    // extra_info + PersonaInject), never by GetAppID.
    //
    // The one exception: while an IClientUserStats IPC call is being dispatched
    // (stats scope active on this thread), the client's stats subsystem reads
    // the "current game" through GetAppIDForCurrentPipe to store/read stats.
    // Without the override it would write under app 480, so the overlay and
    // library would never see unlocks and nothing would persist for the real
    // game. Within the scope only, we translate 480 → real.
    //
    // This mirrors LumaCore's g_userStatsAppIdOverrideDepth override. Every
    // other call path keeps the engine's value untouched, so the pre-9aa4a76
    // "Meccha" regression (leaking real identity into multiplayer routing /
    // friends presence) cannot reappear: the scope is active exclusively on
    // IClientUserStats dispatches.
    if (capture::IsStatsScopeActive()) {
        const AppId realAppId = g_state.onlineFixRealAppId.load(std::memory_order_acquire);
        if (realAppId != 0 && realAppId != constants::kSpacewarAppId &&
            appId == constants::kSpacewarAppId) {
            // Hot path: il gioco chiama GetAppIDForCurrentPipe di continuo;
            // una riga per sessione di gioco basta (DEBUG_ONCE).
            AC_LOG_DEBUG_ONCE(kModule, "GetAppIDForCurrentPipe: stats-scope override %u -> %u.",
                              appId, realAppId);
            return realAppId;
        }
    }

    return appId;
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
