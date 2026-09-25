#include "pch.h"
#include "core/Workers.h"
#include "utils/DeskPaths.h"

#include <algorithm>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <regex>
#include <string>
#include <thread>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "scripting/DirWatch.h"
#include "inject/Diversion.h"
#include "utils/Hasher.h"
#include "core/HookManager.h"
#include "utils/IpcSpec.h"
#include "core/Logger.h"
#include "utils/PatternEngine.h"
#include "scripting/ScriptEngine.h"
#include "core/Settings.h"
#include "diagnostics/StatusWriter.h"
#include "core/SteamVersion.h"
#include "hooks/ipc/PipeWatch.h"
#include "hooks/ipc/CmdUser.h"
#include "hooks/license/LicenseManager.h"
#include "hooks/steamclient/LicenseHooks.h"
#include "hooks/steamclient/OwnershipHooks.h"
#include "hooks/steamui/SteamUIHook.h"
#include "hooks/wire/AchievementBackup.h"
#include "hooks/wire/ManifestRestore.h"
#include "hooks/wire/AchievementModule.h"
#include "network/EticketFetcher.h"

using namespace ac;

namespace {

    constexpr const char* kModule = "Core";

    constexpr const char* kStartupLogToken = "R4M7K1C9";

    // Guards against the (theoretical) possibility of init running twice.
    volatile LONG s_initFlag = 0;

    // ---------------------------------------------------------------------------
    // Initialisation order — every step lists the steps it depends on, so a
    // future maintainer can safely reorder or insert steps without breaking
    // the bootstrap pipeline.  (Audit §3.7, 2026-07-12.)
    //
    //    1. Logger           (depends on: nothing)
    //    2. Settings          (depends on: logger — config errors are logged)
    //       PinSelf           (depends on: nothing; module handle already valid)
    //    3. BuildId           (depends on: nothing; diagnostic only, never fatal)
    //    4. Diversion         (depends on: steamInstallPath from ResolvePaths)
    //    5. steamclient SHA   (depends on: diversion — path resolved)
    //    6. Pattern engine    (depends on: diversion — module handle for hashing)
    //    7. IPC spec          (depends on: pattern engine — patternDir created)
    //    8. Lua scripts       (depends on: nothing; runs standalone sandbox)
    //    9. Hook install      (depends on: diversion + pattern engine + lua maps)
    //   10. DirWatch           (depends on: lua scripts — startup files already
    //                           scanned, so they don't look like hot-reload adds)
    // ---------------------------------------------------------------------------

    // Pin our module by its own load address rather than by file name; this is
    // robust even if the DLL is renamed (improvement over the original).
    void PinSelf() {
        HMODULE pinned = nullptr;
        GetModuleHandleExA(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            reinterpret_cast<LPCSTR>(&PinSelf), &pinned);
        AC_LOG_DEBUG(kModule, "Module pinned in memory.");
    }

    // Resolves all runtime paths from the DLL's own location and creates the
    // aethercore working directory.
    void ResolvePaths(HMODULE self) {
        g_state.selfModule = self;

        char dllPath[MAX_PATH] = {};
        DWORD len = GetModuleFileNameA(self, dllPath, MAX_PATH);
        // GetModuleFileNameA may return a non-null-terminated buffer when the path
        // equals MAX_PATH, so std::string(dllPath) could read past the array.
        if (len == 0 || len >= MAX_PATH) {
            // Path unavailable or truncated. Continue with a best-effort path:
            // the zero-initialised buffer is at least null-terminated at MAX_PATH-1.
            AC_LOG_ERROR(kModule, "Module path unavailable or truncated.");
        }
        dllPath[MAX_PATH - 1] = '\0';
        std::string path(dllPath);
        std::size_t slash = path.find_last_of("\\/");
        g_state.steamInstallPath = (slash == std::string::npos) ? path : path.substr(0, slash);

        g_state.aetherCoreDir = g_state.steamInstallPath + "\\aethercore";
        CreateDirectoryA(g_state.aetherCoreDir.c_str(), nullptr);

        g_state.logFilePath = g_state.aetherCoreDir + "\\main.log";
        g_state.configPath = g_state.aetherCoreDir + "\\aethercore.toml";
        g_state.patternDir = g_state.aetherCoreDir + "\\pattern";
        g_state.payloadDllPath = g_state.steamInstallPath + "\\AetherPayload.dll";

        g_state.deskDataDir = deskpaths::ReadDataDir(g_state.aetherCoreDir);
        if (!g_state.deskDataDir.empty()) {
            g_state.configPath = deskpaths::ConfigPath(g_state.deskDataDir).string();
        }

    }

    // Returns every Steam library's `steamapps` directory. The primary Steam
    // root is always included; secondary libraries are read from
    // steamapps/libraryfolders.vdf so uninstall detection also works when a
    // game lives on another drive.
    std::vector<std::string> CollectAcfWatchDirs() {
        std::vector<std::string> dirs;
        const std::filesystem::path primary =
            std::filesystem::path(g_state.steamInstallPath) / "steamapps";
        dirs.push_back(primary.string());

        const std::filesystem::path libraryFile = primary / "libraryfolders.vdf";
        std::ifstream input(libraryFile);
        if (!input.is_open()) return dirs;
        const std::string content((std::istreambuf_iterator<char>(input)),
                                  std::istreambuf_iterator<char>());
        const std::regex pathRegex(R"REGEX("path"\s+"([^"]+)")REGEX",
                                   std::regex_constants::icase);
        for (std::sregex_iterator it(content.begin(), content.end(), pathRegex), end;
             it != end; ++it) {
            std::string library = (*it)[1].str();
            // VDF escapes Windows separators as `\\\\`.
            std::string::size_type pos = 0;
            while ((pos = library.find("\\\\", pos)) != std::string::npos) {
                library.replace(pos, 2, "\\");
                ++pos;
            }
            const std::filesystem::path steamapps =
                std::filesystem::path(library) / "steamapps";
            if (std::find(dirs.begin(), dirs.end(), steamapps.string()) == dirs.end()) {
                dirs.push_back(steamapps.string());
            }
        }
        return dirs;
    }

    // The real initialisation, run off the loader lock on a dedicated thread.
    // Order matters: every step only depends on those before it.
    void InitThreadLogic(HMODULE self) {
        ResolvePaths(self);

        // 1. Settings: loaded before logger so keep_last_session and log level are known.
        const bool configValid = Settings::Initialize(g_state.configPath);
            const auto settings = Settings::Snapshot();

        // 2. Logger: session-oriented initialisation with backup of previous session.
        log::Init(g_state.logFilePath, settings->logKeepLastSession);
        log::SetLevel(settings->logLevel);

        // 2b. Async status writer: from here on every status::Write() is a
        //     coalesced atomic-counter request, never synchronous disk I/O.
        status::Start();
        if (!configValid) AC_LOG_WARN(kModule, "Startup config missing/invalid: using defaults (%s).",
                                     g_state.configPath.c_str());
        if (g_state.deskDataDir.empty()) {
            AC_LOG_WARN(kModule, "desk_path.cfg missing/empty: using local config; Desk backups disabled for this session.");
        } else {
            AC_LOG_INFO(kModule, "Desk bridge resolved: data=%s config=%s (restart to change bridge).",
                        g_state.deskDataDir.c_str(), g_state.configPath.c_str());
        }
        AC_LOG_INFO(kModule,
            "[%s] AetherCore injected. Steam folder: %s",
            kStartupLogToken, g_state.steamInstallPath.c_str());

        // PinSelf: must run before LoadDiversion so the own module HMODULE is stable.
        PinSelf();

        // 3. Build id detection: diagnostic only, never fatal.
        //    Depends on: nothing (reads a steam.exe export).
        g_state.buildId = DetectSteamBuildId();

        // 4. Diversion: prepare the copy or attach the live client (see [injection]).
        //    Depends on: steamInstallPath from ResolvePaths.
        if (!LoadDiversion()) {
            AC_LOG_ERROR(kModule, "Diversion failed; publishing status and aborting.");
            AC_LOG_INFO(kModule, "Init aborted: diversion failed.");
            status::Write();
            return;
        }

        // 5. steamclient SHA: computed right after diversion so status.json
        //    carries it even if later stages fail.
        //    Depends on: diversion (steamclientPath resolved).
        g_state.steamclientSha = hasher::ComputeFileSha256(g_state.steamclientPath);
        status::Write();

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
        std::thread patternThread([&] {
            patternsOk = pattern::Init();
        });
        std::thread ipcThread([&] {
            ipcspec::Init();
        });
        patternThread.join();
        // Steam beta may have mapped the live steamclient during the pattern
        // fetch. Decide the final target BEFORE the UI redirect and any
        // steamclient hooks: never hook a copy that Steam no longer uses.
        SelectHookTargetBeforeRedirect();
        // In copy mode, arm LoadModuleWithPath immediately, while the IPC
        // lookup and Lua scan are still in progress. This closes the old
        // step-9 window where Steam loaded the live client first.
        ac::hooks::ArmSteamUiRedirectEarly();
        ipcThread.join();
        if (!patternsOk) {
            AC_LOG_WARN(kModule, "Pattern engine produced no tables; some hooks will be skipped.");
        }

        // 8. Lua scripts: populate ownership/depot/token/manifest data so the
        //    first LoadPackage / CheckAppOwnership call sees the full set.
        //    Must run before hooks are installed.
        //    Depends on: nothing (runs a standalone sandboxed interpreter).
        if (!script::Init()) {
            AC_LOG_ERROR(kModule, "Script engine failed to initialise.");
        }

        // 9. Hook install: the optional UI redirect has already been armed
        //    (or is retrying); register the steamclient hooks on the chosen target.
        //    Publishes the final status.json.
        //    Depends on: diversion (module handle) + pattern engine (addresses)
        //                + lua data (maps populated).
        ac::hooks::InstallAllHooks();

        // 9b. Late-pattern retry: if a module pattern table was unavailable at
        //     init (patterns not published yet on a fresh Steam build, offline
        //     start, ...), keep re-probing the sources in the background and,
        //     when a table appears, install the previously-missed hooks
        //     in-session — no Steam restart needed.
        ac::hooks::StartPatternLateRetry();

        // 10. Achievement safety net: snapshot di TUTTI i .bin stats degli app
        //     gestiti (async, una volta per processo). Va il prima possibile:
        //     il login-reconcile del client può scartare i cambi pendenti
        //     (perdita del 21/08) e questa copia deve batterlo sul tempo.
        //     Dipende da: lua data (HasDepot) + aethercore dir (desk_path.cfg).
        ac::hooks::AchievementBackup::BackupAllKnownStatsAtStartup();

        // 10b. Manifest restore: ricopia in Steam\depotcache i .manifest di
        //      backup mancanti (async, una volta per processo). Disinstallare
        //      un gioco cancella i manifest e Steam non li serve più senza
        //      autenticazione — il backup è l'unica copia rimasta. No-op
        //      quando [manifest_cache] restore_on_startup è false.
        //      Dipende da: aethercore dir (desk_path.cfg) + steamInstallPath.
        ac::hooks::ManifestRestore::RestoreMissingManifestsAtStartup();

        // 10. DirWatch: starts the Lua hot-reload watcher so games can be
        //    added/removed without restarting Steam. Runs AFTER the initial
        //    Lua scan so startup files do not look like hot-reload additions.
        //    Depends on: luaDir resolved + luaExtraPaths from settings.
        std::vector<std::string> watchDirs{ g_state.luaDir };
        for (const std::string& extra : settings->luaExtraPaths) watchDirs.push_back(extra);
        // ACF removals are Steam's durable uninstall signal. Keep this watcher
        // separate from the recursive Lua paths so a deleted
        // appmanifest_<app_id>.acf can restore only that app's backed-up
        // manifests into Steam\depotcache after Steam finishes its cleanup.
        const std::vector<std::string> acfWatchDirs = CollectAcfWatchDirs();
        ac::dirwatch::Start(watchDirs, acfWatchDirs);
    }

    DWORD WINAPI InitThread(LPVOID param) {
        InitThreadLogic(static_cast<HMODULE>(param));
        return 0;
    }

    void Shutdown() {
        static std::mutex shutdownMutex;
        static bool completed = false;
        std::lock_guard lock(shutdownMutex);
        if (completed) return;
        g_state.shuttingDown.store(true);
        AC_LOG_INFO(kModule, "Explicit shutdown requested outside loader lock; waiting for initialization.");

        if (g_state.initThread) {
            // This API is never called by DllMain. Do not tear down state while
            // initialization is still publishing it after an arbitrary timeout.
            WaitForSingleObject(g_state.initThread, INFINITE);
            CloseHandle(g_state.initThread);
            g_state.initThread = nullptr;
        }

        status::Stop();
        ac::dirwatch::Stop();
        // Task queue + named workers: stop, drain e join mentre gli hook sono
        // ancora installati (alcuni job one-shot inviano frame via hook).
        ac::workers::Shutdown();
        ac::pipewatch::Reset();
        // Stop the late-pattern retry before the hook/license subsystems go
        // down, so it can never re-arm them mid-shutdown.
        ac::hooks::StopPatternLateRetry();
        ac::hooks::LicenseManager::Shutdown();
        ac::hooks::ShutdownOwnershipHooks();
        ac::hooks::ShutdownLicenseHooks();
        ac::hooks::ShutdownSteamUiRetry();
        ac::hooks::AchievementModule::Shutdown();
        ac::eticketfetch::Shutdown();
        ac::hooks::CmdUser::ResetETicketAsyncCalls();
        g_state.hookManager.UninstallAll();
        script::Shutdown();
        AC_LOG_INFO(kModule, "Explicit shutdown completed; pinned module remains loaded.");
        log::Shutdown();
        completed = true;
    }

}  // namespace

// Terminal service shutdown only. Host must first quiesce game/Steam activity;
// this does NOT establish safe dynamic unloading of detached legacy workers.
extern "C" __declspec(dllexport) void WINAPI AetherCoreShutdown() {
    Shutdown();
}

BOOL APIENTRY DllMain(HMODULE instance, DWORD reason, LPVOID reserved) {
    switch (reason) {
    case DLL_PROCESS_ATTACH:
        DisableThreadLibraryCalls(instance);
        // No heavy work in DllMain (architectural principle 5): hand off to
        // a dedicated thread immediately.
        if (InterlockedCompareExchange(&s_initFlag, 1, 0) == 0) {
            g_state.initThread = CreateThread(nullptr, 0, InitThread, instance, 0, nullptr);
        }
        break;

    case DLL_PROCESS_DETACH:
        // Loader lock: no waits, mutexes, I/O or logging here. Logger flushes
        // every emitted line already. Normal process exit is not an explicit
        // service shutdown and cannot guarantee final achievement snapshots.
        g_state.shuttingDown.store(true, std::memory_order_relaxed);
        break;

    default:
        break;
    }
    return TRUE;
}
