#include "pch.h"

#include <string>
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
#include "hooks/steamui/SteamUIHook.h"

using namespace ac;

namespace {

constexpr const char* kModule = "Core";

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
}

// The real initialisation, run off the loader lock on a dedicated thread.
// Order matters: every step only depends on those before it.
void InitThreadLogic(HMODULE self) {
    ResolvePaths(self);

    // 1. Settings: loaded before logger so keep_last_session and log level are known.
    g_state.settings = Settings::Load(g_state.configPath);

    // 2. Logger: session-oriented initialisation with backup of previous session.
    log::Init(g_state.logFilePath, g_state.settings.logKeepLastSession);
    log::SetLevel(g_state.settings.logLevel);
    AC_LOG_INFO(kModule, "AetherCore injected. Steam folder: %s",
                g_state.steamInstallPath.c_str());

    // PinSelf: must run before LoadDiversion so the own module HMODULE is stable.
    PinSelf();

    // 3. Build id detection: diagnostic only, never fatal.
    //    Depends on: nothing (reads a steam.exe export).
    g_state.buildId = DetectSteamBuildId();

    // 4. Diversion: creates and loads the hookable steamclient copy.
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

    // 6. Pattern engine: resolves hook addresses from per-build TOML tables.
    //    Also fills the steamui SHA. Hook installation later depends on this.
    //    Depends on: diversion (module handle for SHA hashing).
    if (!pattern::Init()) {
        AC_LOG_WARN(kModule, "Pattern engine produced no tables; some hooks will be skipped.");
    }

    // 7. IPC spec: per-build funcHash overrides so IPC dispatch survives
    //    Steam client updates. Must run after pattern::Init() (which creates
    //    the pattern cache directory) and before hook installation (which
    //    registers IPC handlers that consult the spec).
    ipcspec::Init();

    // 8. Lua scripts: populate ownership/depot/token/manifest data so the
    //    first LoadPackage / CheckAppOwnership call sees the full set.
    //    Must run before hooks are installed.
    //    Depends on: nothing (runs a standalone sandboxed interpreter).
    if (!script::Init()) {
        AC_LOG_ERROR(kModule, "Script engine failed to initialise.");
    }

    // 9. Hook install: waits for steamui.dll internally, then registers
    //    every steamclient + steamui hook and enables them atomically.
    //    Publishes the final status.json.
    //    Depends on: diversion (module handle) + pattern engine (addresses)
    //                + lua data (maps populated).
    ac::hooks::InstallAllHooks();

    // 10. DirWatch: starts the Lua hot-reload watcher so games can be
    //    added/removed without restarting Steam. Runs AFTER the initial
    //    Lua scan so startup files do not look like hot-reload additions.
    //    Depends on: luaDir resolved + luaExtraPaths from settings.
    std::vector<std::string> watchDirs{g_state.luaDir};
    for (const std::string& extra : g_state.settings.luaExtraPaths) watchDirs.push_back(extra);
    ac::dirwatch::Start(watchDirs);
}

DWORD WINAPI InitThread(LPVOID param) {
    InitThreadLogic(static_cast<HMODULE>(param));
    return 0;
}

void Shutdown() {
    if (g_state.shuttingDown.exchange(true)) return;
    AC_LOG_INFO(kModule, "Shutting down.");

    if (g_state.initThread) {
        WaitForSingleObject(g_state.initThread, constants::kInitThreadJoinTimeoutMs);
        CloseHandle(g_state.initThread);
        g_state.initThread = nullptr;
    }

    ac::dirwatch::Stop();
    ac::pipewatch::Reset();
    ac::hooks::CmdUser::ResetETicketAsyncCalls();
    g_state.hookManager.UninstallAll();
    script::Shutdown();
    log::Shutdown();
}

}  // namespace

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
            // On process termination (reserved != nullptr) the OS reclaims
            // everything and the loader lock is held, so skip full cleanup.
            // But flush the log so the last diagnostic messages are not lost.
            // (Audit M3, 2026-07-12.)
            if (reserved != nullptr) {
                log::Flush();
            } else {
                Shutdown();
            }
            break;

        default:
            break;
    }
    return TRUE;
}
