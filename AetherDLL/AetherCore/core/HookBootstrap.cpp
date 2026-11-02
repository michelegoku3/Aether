#include "pch.h"
#include "core/HookBootstrap.h"

#include <atomic>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/HookManager.h"
#include "core/Logger.h"
#include "core/Workers.h"
#include "diagnostics/StatusWriter.h"
#include "hooks/aetheronline/CreateProcessHooks.h"
#include "hooks/ipc/IPCBus.h"
#include "hooks/ipc/SteamCapture.h"
#include "hooks/steamclient/AetherOnlineHooks.h"
#include "hooks/steamclient/DecryptionKeyHook.h"
#include "hooks/steamclient/DepotHooks.h"
#include "hooks/steamclient/LicenseHooks.h"
#include "hooks/steamclient/OwnershipHooks.h"
#include "hooks/wire/PacketRouter.h"
#include "utils/GameNameResolver.h"
#include "utils/IpcSpec.h"
#include "utils/PatternEngine.h"
#include "utils/Strings.h"

namespace ac::bootstrap {
namespace {

using ac::hooks::RegisterAetherOnlineHooks;
using ac::hooks::RegisterCreateProcessHooks;
using ac::hooks::RegisterDecryptionKeyHook;
using ac::hooks::RegisterDepotHooks;
using ac::hooks::RegisterIpcBus;
using ac::hooks::RegisterLicenseHooks;
using ac::hooks::RegisterOwnershipHooks;
using ac::hooks::RegisterPacketRouter;

constexpr const char* kModule = "Bootstrap";

std::mutex s_batchMutex;

std::atomic<bool> s_patternRetryStarted{false};
std::atomic<bool> s_patternRetryStop{false};

// Hooks only need the steamclient table and the IPC spec: the steamui table is
// still fetched for diagnostics and future UI work, but nothing installs from it.
bool PatternsFullyAvailable() { return pattern::HasModule("steamclient") && g_state.ipcSpec.loaded; }

void PatternLateRetryWorker(std::atomic<bool>& stop) {
    for (int attempt = 1; attempt <= constants::kPatternLateRetryMaxAttempts; ++attempt) {
        if (stop.load(std::memory_order_relaxed) || s_patternRetryStop.load(std::memory_order_relaxed))
            return;
        if (PatternsFullyAvailable())
            return;

        bool anyTableAppeared = false;
        if (!g_state.ipcSpec.loaded) {
            anyTableAppeared |= ipcspec::Init(); // IPC-only arrival also needs a new batch
        }
        anyTableAppeared |= pattern::ReloadModuleIfMissing("steamclient");

        if (anyTableAppeared) {
            AC_LOG_INFO(kModule, "Late pattern(s) became available; re-running hook batch.");
            InstallAllHooks(); // idempotent: only the previously-missed hooks
                               // register; LicenseManager::Init re-arms A2.
            status::Write();
        } else if (attempt == 1 || attempt % 6 == 0) {
            AC_LOG_INFO(kModule, "Late pattern retry #%d: steamclient=%d ipc=%d.", attempt,
                        static_cast<int>(pattern::HasModule("steamclient")),
                        static_cast<int>(g_state.ipcSpec.loaded.load()));
        }
        if (PatternsFullyAvailable()) {
            AC_LOG_INFO(kModule, "All pattern tables available after retry; "
                                 "in-session hooks installed.");
            return;
        }

        // Event-driven wait on the pattern directory: a freshly published
        // table wakes the next attempt at once instead of at the next 10 s
        // tick. The 250 ms quantum keeps shutdown snappy.
        // UTF-8 in, *W at the boundary (patternDir is UTF-8 narrow).
        const std::wstring wPatternDir = ac::strings::Widen(g_state.patternDir);
        HANDLE change =
            wPatternDir.empty()
                ? INVALID_HANDLE_VALUE
                : FindFirstChangeNotificationW(wPatternDir.c_str(), FALSE,
                                               FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE);
        if (change == INVALID_HANDLE_VALUE) {
            for (int elapsed = 0;
                 elapsed < constants::kPatternLateRetryIntervalMs && !stop.load(std::memory_order_relaxed);
                 elapsed += 250) {
                Sleep(250);
            }
        } else {
            int elapsed = 0;
            while (elapsed < constants::kPatternLateRetryIntervalMs && !stop.load(std::memory_order_relaxed)) {
                const DWORD w = WaitForSingleObject(change, 250);
                elapsed += 250;
                if (w == WAIT_OBJECT_0) {
                    // A table may have landed: retry now instead of at the tick.
                    FindNextChangeNotification(change);
                    break;
                }
            }
            FindCloseChangeNotification(change);
        }
    }
    AC_LOG_WARN(kModule,
                "Late pattern retry gave up after %d attempt(s); "
                "restart Steam once the patterns are published.",
                constants::kPatternLateRetryMaxAttempts);
}

} // namespace

std::mutex& BatchMutex() { return s_batchMutex; }

void InstallSteamClientBatch() {
    std::lock_guard<std::mutex> batchLock(s_batchMutex);
    AC_LOG_INFO(kModule, "Hook batch: registering kernel32 + steamclient hooks.");
    // kernel32.dll hooks (pre-entry payload injection): install before any game
    // can be launched so the first SpawnProcess → CreateProcessW chain is covered.
    RegisterCreateProcessHooks();

    RegisterOwnershipHooks(g_state.steamclientModule);
    RegisterDepotHooks(g_state.steamclientModule);
    RegisterDecryptionKeyHook(g_state.steamclientModule);
    RegisterAetherOnlineHooks(g_state.steamclientModule);

    // IPC layer: capture helpers must resolve before the bus arms, and the bus
    // registers its command handlers internally.
    capture::Init(g_state.steamclientModule);
    RegisterIpcBus(g_state.steamclientModule);

    // Localized titles for presence (game_extra_info / PersonaState game_name).
    // Arms a one-shot capture on GetAppDataFromAppInfo; soft-fails if missing.
    gamename::Init(g_state.steamclientModule);

    // Wire layer: outgoing/incoming packet manipulation (presence included).
    RegisterPacketRouter(g_state.steamclientModule);

    // License/controller compatibility hooks.
    RegisterLicenseHooks(g_state.steamclientModule);

    if (g_state.hookManager.InstallAll()) {
        g_state.hooksInstalled.store(true);
        AC_LOG_INFO(kModule, "Steamclient hooks enabled.");
    } else {
        AC_LOG_ERROR(kModule, "Steamclient hook enable failed.");
    }

    status::Write();
}

void InstallAllHooks() {
    // Idempotent re-run: registers only the previously missed hooks.
    InstallSteamClientBatch();
}

void StartPatternLateRetry() {
    if (PatternsFullyAvailable())
        return; // nothing to retry
    bool expected = false;
    if (!s_patternRetryStarted.compare_exchange_strong(expected, true))
        return;
    s_patternRetryStop.store(false, std::memory_order_relaxed);
    AC_LOG_INFO(kModule,
                "Starting late-pattern retry worker (some pattern tables "
                "were unavailable at init; %d attempts, %d ms apart).",
                constants::kPatternLateRetryMaxAttempts, constants::kPatternLateRetryIntervalMs);
    if (!workers::StartWorker("pattern_late_retry", PatternLateRetryWorker)) {
        s_patternRetryStarted.store(false);
        AC_LOG_ERROR(kModule, "Could not start the late-pattern retry worker; "
                              "missing hooks will need a Steam restart.");
    }
}

void RequestRetryStop() {
    s_patternRetryStop.store(true, std::memory_order_relaxed);
    // The pattern retry owns its stop flag because it may also end on its own
    // once the tables become complete; workers::Shutdown performs the join.
    AC_LOG_INFO(kModule, "Hook retry stop requested; workers::Shutdown performs the join.");
}

} // namespace ac::bootstrap
