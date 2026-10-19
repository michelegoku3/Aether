#include "pch.h"
#include "core/BootSteps.h"

#include <algorithm>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <thread>

#include "core/AetherCoreState.h"
#include "core/AppManifestOwner.h"
#include "core/Constants.h"
#include "core/HookBootstrap.h"
#include "core/HookManager.h"
#include "core/Logger.h"
#include "core/NetPacketAbi.h"
#include "core/Settings.h"
#include "core/SteamVersion.h"
#include "core/StructGuard.h"
#include "diagnostics/StatusWriter.h"
#include "hooks/license/LicenseManager.h"
#include "hooks/wire/AchievementBackup.h"
#include "hooks/wire/ManifestRestore.h"
#include "inject/Diversion.h"
#include "scripting/DirWatch.h"
#include "scripting/ScriptEngine.h"
#include "utils/DeskPaths.h"
#include "utils/Hasher.h"
#include "utils/IpcSpec.h"
#include "utils/PatternEngine.h"
#include "utils/Strings.h"
#include "utils/SteamLibraryPaths.h"
#include "utils/VdfText.h"
#include "utils/WinFile.h"

namespace ac::boot {
namespace {

// Same log tag as the historical InitThreadLogic ("Core"): log bytes must
// not change with this move.
constexpr const char* kModule = "Core";

constexpr const char* kStartupLogToken = "R4M7K1C9";

// Pin our module by its own load address rather than by file name; this is
// robust even if the DLL is renamed (improvement over the original).
void PinSelf() {
    HMODULE pinned = nullptr;
    GetModuleHandleExA(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
                       reinterpret_cast<LPCSTR>(&PinSelf), &pinned);
    AC_LOG_DEBUG(kModule, "Module pinned in memory.");
}

// Returns every Steam library's `steamapps` directory. The primary Steam
// root is always included; secondary libraries are read from
// steamapps/libraryfolders.vdf so uninstall detection also works when a
// game lives on another drive. Shared with the appmanifest owner maintenance
// (utils/SteamLibraryPaths.h) so both see exactly the same library set.
std::vector<std::string> CollectAcfWatchDirs() {
    return steamlibrary::SteamAppsDirs(g_state.steamInstallPath);
}

} // namespace

// Initialisation order (moved verbatim from InitThreadLogic; every step
// lists the steps it depends on):
//    Resolve  paths           (depends on: DLL handle)
//    1. Settings               (depends on: logger — config errors are logged)
//       PinSelf                (depends on: nothing; module handle already valid)
//    3. BuildId                (depends on: nothing; diagnostic only, never fatal)
//    4. Diversion              (depends on: steamInstallPath from ResolvePaths)
//    5. steamclient SHA        (depends on: diversion — path resolved)
//    6. Pattern engine         (depends on: diversion — module handle for hashing)
//    7. IPC spec               (depends on: pattern engine — patternDir created)
//    8. Lua scripts            (depends on: nothing; runs standalone sandbox)
//    9. Hook install           (depends on: diversion + pattern engine + lua maps)
//   10. DirWatch               (depends on: lua scripts — startup files already
//                              scanned, so they don't look like hot-reload adds)

bool ResolveStage(HMODULE self, BootContext& ctx) {
    (void)ctx;
    g_state.selfModule = self;

    // Wide API with a 32 KiB buffer: MAX_PATH truncation and non-ASCII Steam
    // installs (system-codepage mangling via *A) are both gone. Internal
    // plumbing stays UTF-8 narrow; *W is used only at the Win32 boundary.
    std::wstring wide(32768, L'\0');
    DWORD len = GetModuleFileNameW(self, wide.data(), static_cast<DWORD>(wide.size()));
    if (len == 0 || len >= wide.size()) {
        AC_LOG_ERROR(kModule, "Module path unavailable or truncated; aborting init (no directory writes attempted).");
        return false;
    }
    const std::string path = ac::strings::Narrow(std::wstring_view(wide.data(), len));
    if (path.empty()) {
        AC_LOG_ERROR(kModule, "Module path is not valid UTF-16; aborting init.");
        return false;
    }
    std::size_t slash = path.find_last_of("\\/");
    g_state.steamInstallPath = (slash == std::string::npos) ? path : path.substr(0, slash);

    g_state.aetherCoreDir = g_state.steamInstallPath + "\\aethercore";
    winfile::CreateDir(g_state.aetherCoreDir);

    g_state.logFilePath = g_state.aetherCoreDir + "\\main.log";
    g_state.configPath = g_state.aetherCoreDir + "\\aethercore.toml";
    g_state.patternDir = g_state.aetherCoreDir + "\\pattern";
    g_state.payloadDllPath = g_state.steamInstallPath + "\\AetherPayload.dll";

    g_state.deskDataDir = deskpaths::ReadDataDir(g_state.aetherCoreDir);
    if (!g_state.deskDataDir.empty()) {
        g_state.configPath = deskpaths::ConfigPath(g_state.deskDataDir).string();
    }
    return true;
}

bool InitLoggingStage(BootContext& ctx) {
    // 1. Settings: loaded before logger so keep_last_session and log level are known.
    ctx.configValid = Settings::Initialize(g_state.configPath);
    ctx.settings = Settings::Snapshot();

    // 2. Logger: session-oriented initialisation with backup of previous session.
    log::Init(g_state.logFilePath, ctx.settings->logKeepLastSession);
    log::SetLevel(ctx.settings->logLevel);

    // 2b. Async status writer: from here on every status::Write() is a
    //     coalesced atomic-counter request, never synchronous disk I/O.
    status::Start();
    if (!ctx.configValid)
        AC_LOG_WARN(kModule, "Startup config missing/invalid: using defaults (%s).", g_state.configPath.c_str());
    if (g_state.deskDataDir.empty()) {
        AC_LOG_WARN(kModule,
                    "desk_path.cfg missing/empty: using local config; Desk backups disabled for this session.");
    } else {
        AC_LOG_INFO(kModule, "Desk bridge resolved: data=%s config=%s (restart to change bridge).",
                    g_state.deskDataDir.c_str(), g_state.configPath.c_str());
    }
    AC_LOG_INFO(kModule, "[%s] AetherCore injected. Steam folder: %s", kStartupLogToken,
                g_state.steamInstallPath.c_str());

    // PinSelf: must run before LoadDiversion so the own module HMODULE is stable.
    PinSelf();
    return true;
}

bool InitDiversionStage() {
    // 3. Build id detection: diagnostic only, never fatal.
    //    Depends on: nothing (reads a steam.exe export).
    g_state.buildId = DetectSteamBuildId();

    // 4. Diversion: prepare the copy or attach the live client (see [injection]).
    //    Depends on: steamInstallPath from ResolvePaths.
    if (!LoadDiversion()) {
        AC_LOG_ERROR(kModule, "Diversion failed; publishing status and aborting.");
        AC_LOG_INFO(kModule, "Init aborted: diversion failed.");
        status::Write();
        return false;
    }

    // 5. steamclient SHA: computed right after diversion so status.json
    //    carries it even if later stages fail.
    //    Depends on: diversion (steamclientPath resolved).
    g_state.steamclientSha = hasher::ComputeFileSha256(g_state.steamclientPath);
    status::Write();
    return true;
}

bool InitPatternsStage(BootContext& ctx) {
    (void)ctx;
    // 6. Pattern engine + 7. IPC spec, resolved CONCURRENTLY.
    //    The pattern engine resolves hook addresses from per-build TOML
    //    tables (steamclient + steamui in parallel internally); the IPC spec
    //    fetches the per-build funcHash table. Both can miss the cache on a
    //    fresh Steam build and block on network round-trips, so running them
    //    side by side keeps hook installation early enough to catch Steam's
    //    one-shot startup events (LoadPackage of package 0). The downloader
    //    creates any cache subdirectory it needs, so there is no ordering
    //    dependency between the two.
    bool patternsOk = false;
    std::thread patternThread([&] { patternsOk = pattern::Init(); });
    std::thread ipcThread([&] { ipcspec::Init(); });
    patternThread.join();
    // Steam beta may have mapped the live steamclient during the pattern
    // fetch. Decide the final target BEFORE the UI redirect and any
    // steamclient hooks: never hook a copy that Steam no longer uses.
    SelectHookTargetBeforeRedirect();
    // In copy mode, arm LoadModuleWithPath immediately, while the IPC
    // lookup and Lua scan are still in progress. This closes the old
    // step-9 window where Steam loaded the live client first.
    ac::bootstrap::ArmSteamUiRedirect();
    ipcThread.join();
    if (!patternsOk) {
        AC_LOG_WARN(kModule, "Pattern engine produced no tables; some hooks will be skipped.");
    }
    return patternsOk;
}

void SeedAbiStage() {
    // Phase 4: the build's own ABI table proposes the CNetPacket layout.
    // It runs BEFORE SeedFromBuild so a published table always wins over
    // the two layouts compiled into this DLL, and before any hook is
    // installed so the first packet can already be interpreted.
    {
        std::uint32_t dataOff = 0, cubOff = 0, refOff = 0;
        std::string abiSource;
        {
            std::lock_guard lock(g_state.abiTable.mutex);
            const auto& offsets = g_state.abiTable.offsets;
            const auto data = offsets.find("CNetPacket.m_pubData");
            const auto cub = offsets.find("CNetPacket.m_cubData");
            const auto ref = offsets.find("CNetPacket.m_cRef");
            if (data != offsets.end() && cub != offsets.end() && ref != offsets.end()) {
                dataOff = data->second;
                cubOff = cub->second;
                refOff = ref->second;
                abiSource = g_state.abiTable.source;
            }
        }
        if (dataOff != 0) {
            abi::netpkt::SeedFromAbiTable(dataOff, cubOff, refOff, abiSource);
        }

        // Same table, other structs: here we cannot re-lay-out anything,
        // so the table acts as a verifier. A contradiction disables the
        // guarded writes instead of corrupting Steam quietly.
        {
            std::lock_guard lock(g_state.abiTable.mutex);
            if (!g_state.abiTable.offsets.empty()) {
                abi::guard::ApplyTable(g_state.abiTable.offsets, g_state.abiTable.source);
            }
        }
    }

    // Fallback for machines with no table at all: the two builds whose
    // layout is compiled in. Skipped when the table already spoke.
    abi::netpkt::SeedFromBuild(g_state.steamclientSha);
}

bool InitLuaStage() {
    // 8. Lua scripts: populate ownership/depot/token/manifest data so the
    //    first LoadPackage / CheckAppOwnership call sees the full set.
    //    Must run before hooks are installed.
    //    Depends on: nothing (runs a standalone sandboxed interpreter).
    if (!script::Init()) {
        AC_LOG_ERROR(kModule, "Script engine failed to initialise.");
        return false;
    }
    return true;
}

void InstallHooksStage() {
    // 9. Hook install: the optional UI redirect has already been armed
    //    (or is retrying); register the steamclient hooks on the chosen target.
    //    Publishes the final status.json.
    //    Depends on: diversion (module handle) + pattern engine (addresses)
    //                + lua data (maps populated).
    ac::bootstrap::InstallAllHooks();

    // 9b. Late-pattern retry: if a module pattern table was unavailable at
    //     init (patterns not published yet on a fresh Steam build, offline
    //     start, ...), keep re-probing the sources in the background and,
    //     when a table appears, install the previously-missed hooks
    //     in-session — no Steam restart needed.
    ac::bootstrap::StartPatternLateRetry();
}

void StartSafetyNetsStage() {
    // 10. Achievement safety net: snapshot of ALL managed apps' stats .bins
    //     (async, once per process). As early as possible: the client's
    //     login-reconcile may discard pending changes (the 21/08 loss) and
    //     this copy must beat it to it.
    //     Depends on: lua data (HasDepot) + aethercore dir (desk_path.cfg).
    ac::hooks::AchievementBackup::BackupAllKnownStatsAtStartup();

    // 10b. Manifest restore: recopies missing backup .manifests into
    //      Steam\depotcache (async, once per process). Uninstalling a game
    //      deletes manifests and Steam no longer serves them without
    //      authentication — the backup is the only copy left. No-op when
    //      [manifest_cache] restore_on_startup is false.
    //      Depends on: aethercore dir (desk_path.cfg) + steamInstallPath.
    ac::hooks::ManifestRestore::RestoreMissingManifestsAtStartup();

    // 10c. Owner strip: Steam derives the "From the library of <account>"
    //      banner from AppState.LastOwner, the account that last
    //      installed/updated the app. Lua apps are installed by whichever
    //      account was active at the time, so a later login shows a bogus
    //      shared-library banner. The entry is dropped (never stamped: the
    //      incoming account is unknown before login), which leaves Steam with
    //      nobody to attribute the app to. Runs before Steam builds this
    //      session's app list (async, once per process); MarkLicenseAsChanged
    //      and DirWatch repeat it on login and on every manifest rewrite.
    //      Depends on: lua data (HasDepot) + steamInstallPath.
    ac::appmanifest::StripAtStartup();
}

void StartWatchersStage(const BootContext& ctx) {
    // 10. DirWatch: starts the Lua hot-reload watcher so games can be
    //    added/removed without restarting Steam. Runs AFTER the initial
    //    Lua scan so startup files do not look like hot-reload additions.
    //    Depends on: luaDir resolved + luaExtraPaths from settings.
    std::vector<std::string> watchDirs{g_state.luaDir};
    for (const std::string& extra : ctx.settings->luaExtraPaths)
        watchDirs.push_back(extra);
    // ACF removals are Steam's durable uninstall signal. Keep this watcher
    // separate from the recursive Lua paths so a deleted
    // appmanifest_<app_id>.acf can restore only that app's backed-up
    // manifests into Steam\depotcache after Steam finishes its cleanup.
    const std::vector<std::string> acfWatchDirs = CollectAcfWatchDirs();
    ac::dirwatch::Start(watchDirs, acfWatchDirs);
}

void InitThreadLogic(HMODULE self) {
    BootContext ctx;
    if (!ResolveStage(self, ctx)) {
        // No paths, no directories, no status: anything else would operate
        // on a truncated prefix. Logger is not up yet by design (it needs
        // the log path), so the ERROR above is the whole report.
        return;
    }
    InitLoggingStage(ctx);
    if (!InitDiversionStage())
        return;
    // Non-fatal: missing tables only degrade gracefully (some hooks are
    // skipped, the late-pattern retry covers them). Carried for a future
    // strict mode that could abort here instead of limping on.
    ctx.patternsOk = InitPatternsStage(ctx);
    SeedAbiStage();
    InitLuaStage();
    InstallHooksStage();
    StartSafetyNetsStage();
    StartWatchersStage(ctx);
}

} // namespace ac::boot
