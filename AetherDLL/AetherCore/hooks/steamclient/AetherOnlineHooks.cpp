#include "pch.h"
#include "hooks/steamclient/AetherOnlineHooks.h"

#include <algorithm>
#include <cstring>
#include <fstream>
#include <filesystem>
#include <sstream>
#include <string>

#include "core/AetherCoreState.h"
#include "hooks/aetheronline/PresenceSession.h"
#include "core/Constants.h"
#include "core/HookManager.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "core/SteamTypes.h"
#include "hooks/ipc/SteamCapture.h"
#include "hooks/wire/BackupIo.h"
#include "utils/VdfText.h"

// ---------------------------------------------------------------------------
// AetherOnline — la modalità online PROPRIA di Aether (payload AetherDLL +
// mask 480, token "-aetheronline"). NON è la crack online-fix.me (OFME):
// i file OFME sul disco sono RILEVATI da AetherDesk (online/foreign.rs), mai
// lanciati o gestiti da questo modulo. I due stack non convivono in un
// processo. Vocabolario completo: AGENTS.md in radice.
// ---------------------------------------------------------------------------

namespace ac::hooks {
namespace {

constexpr const char* kModule = "AetherOnline";
using namespace ac::steam;

// pGameID points at a uint64 GameID whose low 24 bits hold the AppId.
using SpawnProcess_t = bool (*)(void*, const char*, const char*, const char*, std::uint64_t*,
                               const void*, std::uint32_t, std::int32_t);
using GetAppIDForCurrentPipe_t = AppId (*)(void*);

SpawnProcess_t o_SpawnProcess = nullptr;
GetAppIDForCurrentPipe_t o_GetAppIDForCurrentPipe = nullptr;

// Checks whether cmdLine contains 'flag' as a whole argument
// (space-delimited), not as a substring. strstr() would match "-aetheronline2"
// or "--aetheronline" which is incorrect — only the exact argument triggers
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

// AetherOnline token: "-aetheronline" — Aether's own masked-online mode,
// never confused with the online-fix.me (OFME) crack files.
static bool HasAetherOnlineFlag(const char* cmdLine) {
    return HasFlagArg(cmdLine, constants::kAetherOnlineFlag);
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
// one, a -showonline presence session, or a -aetheronline masked session?
// The TOML arrays in [presence] are the source of truth; the legacy argv
// tokens keep working for unmigrated configs but are always stripped from
// the child command line below.
enum class LaunchMode : std::uint8_t { None, ShowOnline, AetherOnline };

static bool InAppList(const std::vector<std::uint32_t>& list, AppId app) {
    return std::find(list.begin(), list.end(), static_cast<std::uint32_t>(app)) != list.end();
}

// Precedence (documented): exclude > aetheronline > showonline > default_mode.
// exclude is a HARD opt-out: it beats even a forgotten legacy argv token.
static LaunchMode ResolveLaunchMode(AppId app, bool aetherOnlineToken, bool showOnlineToken,
                                    const char** sourceOut) {
                                        const auto settings = Settings::Snapshot();
    const auto& s = *settings;
    *sourceOut = "none";
    if (InAppList(s.presenceExcludeApps, app)) {
        *sourceOut = "exclude_apps (hard opt-out)";
        return LaunchMode::None;
    }
    if (InAppList(s.presenceAetherOnlineApps, app)) {
        *sourceOut = "aetheronline_apps";
        return LaunchMode::AetherOnline;
    }
    if (InAppList(s.presenceShowOnlineApps, app)) {
        *sourceOut = "showonline_apps";
        return LaunchMode::ShowOnline;
    }
    // Global custom presence name (docs/05 §13): when set, EVERY app without
    // an explicit entry behaves like a -showonline session — presence-only,
    // never the AetherOnline mask. The name itself already overrides the display
    // in BOTH pipelines via DisplayName() (GamesPlayedModule), so -aetheronline
    // games also show the custom name while keeping their mask.
    if (!s.presenceCustomGameName.empty()) {
        *sourceOut = "custom_game_name (global presence override)";
        return LaunchMode::ShowOnline;
    }
    if (aetherOnlineToken) {
        *sourceOut = "-aetheronline argv token";
        return LaunchMode::AetherOnline;
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

// Returns cmdLine minus every Aether control token (-aetheronline / -showonline).
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
        if (tok == constants::kAetherOnlineFlag || tok == constants::kShowOnlineFlag) {
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

    // Language is read with the shared VDF codec (utils/VdfText.h): the ACF
    // files are VDF text, and this closes the last regex-based VDF parser
    // (P11). Case-insensitive, unescapes `\\\\` pairs like Steam's own reader.
    std::string realContent;
    {
        std::ifstream realFile(realAcf);
        if (!realFile.is_open()) {
            AC_LOG_DEBUG(kModule, "SyncLanguage: cannot open %s.", realAcf.c_str());
            return;
        }
        realContent.assign((std::istreambuf_iterator<char>(realFile)),
                            std::istreambuf_iterator<char>());
    }
    const auto languages = vdf::ExtractQuotedValues(realContent, "language");
    if (languages.empty()) {
        AC_LOG_DEBUG(kModule, "SyncLanguage: no language field in appmanifest_%u.acf.", realAppId);
        return;
    }
    const std::string language = languages.front();
    if (language.empty()) return;
    // A value containing quotes/backslashes cannot round-trip through the
    // hand-written insert below: refuse instead of risking a malformed ACF.
    if (language.find_first_of("\"\\") != std::string::npos) {
        AC_LOG_WARN(kModule, "SyncLanguage: language '%s' contains unsupported "
                             "characters; 480 ACF left untouched.", language.c_str());
        return;
    }

    AC_LOG_INFO(kModule, "SyncLanguage: app %u language='%s'.", realAppId, language.c_str());

    std::string swContent;
    {
        std::ifstream swFile(swAcf);
        if (swFile.is_open()) {
            swContent.assign((std::istreambuf_iterator<char>(swFile)),
                              std::istreambuf_iterator<char>());
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
    } else if (!vdf::ExtractQuotedValues(swContent, "language").empty()) {
        // Replace the value of the first "language" "<value>" pair.
        // Local scan instead of regex: find the quoted key (case-insensitive),
        // then the quoted value after it, and swap only the value span.
        const std::string needle = "\"language\"";
        std::size_t keyPos = std::string::npos;
        for (std::size_t pos = 0; pos + needle.size() <= swContent.size(); ++pos) {
            bool same = true;
            for (std::size_t i = 0; same && i < needle.size(); ++i) {
                const char a = swContent[pos + i], b = needle[i];
                same = (a == b) ||
                       (a >= 'A' && a <= 'Z' && static_cast<char>(a - 'A' + 'a') == b);
            }
            if (same) { keyPos = pos; break; }
        }
        const std::size_t vOpen = keyPos == std::string::npos ? std::string::npos
                                  : swContent.find('"', keyPos + needle.size());
        const std::size_t vStart = vOpen == std::string::npos ? std::string::npos
                                   : swContent.find('"', vOpen + 1);
        const std::size_t vEnd = vStart == std::string::npos ? std::string::npos
                                 : swContent.find('"', vStart + 1);
        if (vStart != std::string::npos && vEnd != std::string::npos) {
            swContent.replace(vStart + 1, vEnd - vStart - 1, language);
        } else {
            AC_LOG_WARN(kModule, "SyncLanguage: malformed language field in the 480 ACF; file left untouched.");
            return;
        }
    } else {
        // Language field missing — insert it inside UserConfig when present,
        // else just before the last closing brace.
        const std::string line = "\n\t\t\"language\"\t\t\"" + language + "\"";
        const auto ucPos = swContent.find("\"UserConfig\"");
        if (ucPos != std::string::npos) {
            const auto bracePos = swContent.find('{', ucPos);
            if (bracePos != std::string::npos) {
                swContent.insert(bracePos + 1, line);
            }
        } else {
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

    // Atomic write (tmp + rename): a crash mid-write can no longer leave a
    // corrupted appmanifest_480.acf behind.
    const std::string tmpPath = swAcf + ".aether-tmp";
    {
        std::ofstream outFile(tmpPath, std::ios::trunc);
        if (!outFile.is_open()) {
            AC_LOG_WARN(kModule, "SyncLanguage: cannot write %s.", tmpPath.c_str());
            return;
        }
        outFile << swContent;
        outFile.flush();
        if (!outFile.good()) {
            outFile.close();
            DeleteFileA(tmpPath.c_str());
            AC_LOG_WARN(kModule, "SyncLanguage: write failed for %s.", tmpPath.c_str());
            return;
        }
    }
    if (!backup::io::AtomicReplace(tmpPath, swAcf)) {
        std::error_code cleanupEc;
        std::filesystem::remove(tmpPath, cleanupEc);
        AC_LOG_WARN(kModule, "SyncLanguage: cannot atomically replace appmanifest_480.acf.");
        return;
    }
    AC_LOG_INFO(kModule, "SyncLanguage: wrote language='%s' to appmanifest_480.acf (atomic).",
                language.c_str());
}

bool h_SpawnProcess(void* user, const char* exe, const char* cmdLine, const char* workDir,
                    std::uint64_t* gameId, const void* blob, std::uint32_t blobSize,
                    std::int32_t launchOption) {
    std::string childCmdStorage;
    const char* childCmd = cmdLine;
    if (gameId) {
        // ResolveLaunchMode retains the latest published snapshot. File polling
        // belongs to DirWatch, never to this process-creation hook.

        AppId realApp = static_cast<AppId>(*gameId & constants::kGameIdAppIdMask);
        if (realApp != 0 && realApp != constants::kSpacewarAppId) {
            g_state.lastSpawnedAppId.store(realApp);
        } else if (realApp == constants::kSpacewarAppId) {
            // A 480 launch is the foreign-crack (UCO2/OFME) signature — or the
            // user playing Spacewar itself. A real appid left over from an
            // earlier launch must not be attributed to it: the spoofed
            // session is named from its live pipe image instead
            // (GamesPlayed::RealAppForSpoofedSession).
            g_state.lastSpawnedAppId.store(0);
        }

        // Centralised resolution (docs/05 §12): TOML arrays are the source of
        // truth; argv tokens (-aetheronline / -showonline) are LEGACY hints that
        // keep working for unmigrated configs and are ALWAYS stripped from
        // the child command line below. exclude_apps wins over tokens too.
        const bool hasAetherOnlineToken = HasAetherOnlineFlag(cmdLine);
        const bool hasSoToken = HasShowOnlineFlag(cmdLine);
        const char* modeSource = nullptr;
        LaunchMode mode = ResolveLaunchMode(realApp, hasAetherOnlineToken, hasSoToken, &modeSource);
        const bool spoofOnDisk = HasSpacewarSpoofOnDisk(exe, workDir);
        presence::SessionSnapshot session;
        session.spacewarSpoofExpected = spoofOnDisk;
        if (mode == LaunchMode::ShowOnline && spoofOnDisk) {
            mode = LaunchMode::None;
            modeSource = "UCO2/OFME on disk; skip showonline";
        }
        // UCO2/OFME launched FROM THE LIBRARY: the client already registered
        // this process under its real appid, and a foreign crack cannot re-key
        // it afterwards — UCO2's Spacewar spoof only applies to the
        // process-originated Launch (direct exe / steam_appid.txt), which a
        // library launch never performs. UCO2 would then announce the real
        // identity on the wire, which breaks the Spacewar-based invite system
        // (lobbies and invites are keyed on 480). Apply the same 480 process
        // mask AetherOnline uses, WITHOUT any Online Aether state: the foreign
        // crack owns the process and the networking (UCO2's AppId=480 ini
        // matches the mask; ogAppId keeps DLC/tickets/stats on the real app),
        // Aether only supplies the Spacewar identity. The real appid is
        // already recorded in lastSpawnedAppId above for the wire naming.
        const bool spacewarMask = spoofOnDisk && mode != LaunchMode::AetherOnline &&
                                  realApp != 0 && realApp != constants::kSpacewarAppId;
        if (spacewarMask) modeSource = "UCO2/OFME on disk; Spacewar mask";
        // Verdict line for EVERY launch (including mode None / depot misses,
        // which the branches below leave silent): with the build stamp this
        // tells you exactly what the DLL decided and why — marker read vs
        // argv token vs default — instead of guessing after the fact.
        AC_LOG_INFO(kModule,
                    "Presence resolve: app %u -> %s (source: %s; argv aetheronline=%d showonline=%d; depot=%d).",
                    realApp,
                    mode == LaunchMode::AetherOnline ? "aetheronline"
                        : mode == LaunchMode::ShowOnline ? "showonline" : "none",
                    modeSource ? modeSource : "unknown",
                    hasAetherOnlineToken ? 1 : 0, hasSoToken ? 1 : 0,
                    luadata::HasDepot(realApp) ? 1 : 0);

        // Hand the child process a clean argv whenever an Aether token was
        // present (see StripAetherFlagArgs). Never null; empty string only
        // when the token was the whole command line.
        if (hasAetherOnlineToken || hasSoToken) {
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

        if (mode == LaunchMode::AetherOnline && luadata::HasDepot(realApp)) {
            // AetherOnline: full 480 process mask — a strict superset of what
            // -showonline needs (server presence + friend notification), and
            // the mask is what real multiplayer through a crack requires.
            session.realAppId = realApp;
            *gameId = (*gameId & ~constants::kGameIdAppIdMask) | constants::kSpacewarAppId;
            AC_LOG_INFO(kModule,
                        "Masked AppId %u as Spacewar (%u) for AetherOnline (source: %s).",
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
            session.showOnlineAppId = realApp;
            AC_LOG_INFO(kModule,
                        "ShowOnline session for app %u: process NOT masked; "
                        "wire-level presence rewrite only (source: %s).",
                        realApp, modeSource);
        } else if (spacewarMask) {
            // UCO2/OFME library launch: same 480 registration as AetherOnline,
            // but NO aetherOnlineRealAppId — that would arm the Online Aether
            // payload injection (OnlinePayload::MaybeInject, CreateProcess
            // hooks) and the AetherOnline-only IPC translations inside a process
            // the foreign crack already owns.
            *gameId = (*gameId & ~constants::kGameIdAppIdMask) | constants::kSpacewarAppId;
            AC_LOG_INFO(kModule,
                        "Masked AppId %u as Spacewar (%u) for UCO2/OFME launch "
                        "(source: %s).",
                        realApp, constants::kSpacewarAppId, modeSource);
            // Same language fix as AetherOnline: the client reads the 480 ACF.
            SyncLanguageToSpacewar(realApp);
        }
        // Unica pubblicazione atomica di tutti i campi: i lettori non possono
        // osservare stati intermedi tra i vecchi store separati.
        presence::Publish(session);
    }
    return o_SpawnProcess(user, exe, childCmd, workDir, gameId, blob, blobSize, launchOption);
}

AppId h_GetAppIDForCurrentPipe(void* engine) {
    void* prev = nullptr;
    if (g_state.steamEngine.compare_exchange_strong(prev, engine)) {
        AC_LOG_INFO(kModule, "Captured steamEngine pointer 0x%p.", engine);
    }

    AppId appId = o_GetAppIDForCurrentPipe(engine);

    // Scoped AetherOnline stats override (see capture::EnterStatsScope).
    //
    // AetherOnline masks the process as Spacewar/480 for multiplayer routing. The
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
        const AppId realAppId = presence::RealAppId();
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
    AppId realAppId = presence::RealAppId();

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

void RegisterAetherOnlineHooks(HMODULE diversion) {
    if (!diversion) {
        AC_LOG_ERROR(kModule, "Diversion module not loaded.");
        return;
    }
    AC_LOG_INFO(kModule, "Registering AetherOnline hooks.");
    g_state.hookManager.TryHook("SpawnProcess", "steamclient", diversion,
                          o_SpawnProcess, h_SpawnProcess);
    g_state.hookManager.TryHook("GetAppIDForCurrentPipe", "steamclient", diversion,
                          o_GetAppIDForCurrentPipe, h_GetAppIDForCurrentPipe);
    g_state.hookManager.TryHook("BuildSpawnEnvBlock", "steamclient", diversion,
                          o_BuildSpawnEnvBlock, h_BuildSpawnEnvBlock);
}

}  // namespace ac::hooks
