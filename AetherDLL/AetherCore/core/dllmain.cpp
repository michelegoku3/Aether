#include "pch.h"
#include "core/BootSteps.h"
#include "core/Workers.h"

#include <atomic>
#include <mutex>

#include "core/AetherCoreState.h"
#include "core/HookBootstrap.h"
#include "core/Logger.h"
#include "diagnostics/StatusWriter.h"
#include "hooks/ipc/CmdUser.h"
#include "hooks/ipc/PipeWatch.h"
#include "hooks/license/LicenseManager.h"
#include "hooks/steamclient/LicenseHooks.h"
#include "hooks/steamclient/OwnershipHooks.h"
#include "hooks/wire/AchievementModule.h"
#include "network/EticketFetcher.h"
#include "scripting/DirWatch.h"
#include "scripting/ScriptEngine.h"

using namespace ac;

namespace {

constexpr const char* kModule = "Core";

// Guards against the (theoretical) possibility of init running twice.
// std::atomic (not volatile LONG): volatile does not order memory in C++;
// the atomic compare-exchange is the single-execution guarantee.
std::atomic<long> s_initFlag{0};

// The real initialisation lives in core/BootSteps (one function per stage);
// this TU owns only the loader-lock boundary, the init thread and the
// explicit terminal shutdown below.
DWORD WINAPI InitThread(LPVOID param) {
    ac::boot::InitThreadLogic(static_cast<HMODULE>(param));
    return 0;
}

void Shutdown() {
    static std::mutex shutdownMutex;
    static bool completed = false;
    std::lock_guard lock(shutdownMutex);
    if (completed)
        return;
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
    // Task queue + named workers: stop, drain and join while hooks are still
    // installed (some one-shot jobs send frames via hook).
    ac::workers::Shutdown();
    ac::pipewatch::Reset();
    // Retry hooks (steamui redirect + late patterns) are workers: the stop
    // requested here and the join inside workers::Shutdown() already
    // happened above, so no re-arm is possible during shutdown.
    ac::bootstrap::RequestRetryStop();
    ac::hooks::LicenseManager::Shutdown();
    ac::hooks::ShutdownOwnershipHooks();
    ac::hooks::ShutdownLicenseHooks();
    ac::hooks::AchievementModule::Shutdown();
    ac::eticketfetch::Shutdown();
    ac::hooks::CmdUser::ResetETicketAsyncCalls();
    g_state.hookManager.UninstallAll();
    script::Shutdown();
    AC_LOG_INFO(kModule, "Explicit shutdown completed; pinned module remains loaded.");
    log::Shutdown();
    completed = true;
}

} // namespace

// Terminal service shutdown only. Host must first quiesce game/Steam activity;
// this does NOT establish safe dynamic unloading of detached legacy workers.
extern "C" __declspec(dllexport) void WINAPI AetherCoreShutdown() { Shutdown(); }

BOOL APIENTRY DllMain(HMODULE instance, DWORD reason, LPVOID /*reserved*/) {
    switch (reason) {
    case DLL_PROCESS_ATTACH: {
        DisableThreadLibraryCalls(instance);
        // No heavy work in DllMain (architectural principle 5): hand off to
        // a dedicated thread immediately.
        long expected = 0;
        if (s_initFlag.compare_exchange_strong(expected, 1, std::memory_order_acq_rel)) {
            g_state.initThread = CreateThread(nullptr, 0, InitThread, instance, 0, nullptr);
        }
        break;
    }

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
