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
#include "hooks/steamui/AppOverviewHooks.h"
#include "hooks/wire/PacketRouter.h"
#include "inject/SteamclientAttach.h"
#include "inject/SteamuiAttach.h"
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
std::atomic<bool> s_uiBatchInstalled{false};
std::atomic<bool> s_uiAttachRetryStarted{false};

// Hooks only need the steamclient table and the IPC spec; the steamui table
// feeds the app-overview batch, installed separately when that module maps.
// The module handle is part of readiness: without it there is nothing to hook,
// and the retry keeps waiting for Steam to map its client.
bool ReadyToInstallHooks() {
    return g_state.steamclientModule != nullptr && pattern::HasModule("steamclient") && g_state.ipcSpec.loaded;
}

void PatternLateRetryWorker(std::atomic<bool>& stop) {
    for (int attempt = 1; attempt <= constants::kPatternLateRetryMaxAttempts; ++attempt) {
        if (stop.load(std::memory_order_relaxed) || s_patternRetryStop.load(std::memory_order_relaxed))
            return;
        if (ReadyToInstallHooks())
            return;

        bool anyTableAppeared = false;
        // Steam may map its client after the hook batch gave up; re-probe before
        // deciding whether there is anything new to install. An attach that
        // already happened is not news and must not re-run the batch by itself.
        if (g_state.steamclientModule == nullptr)
            anyTableAppeared |= AttachSteamclient(0);
        if (!g_state.ipcSpec.loaded) {
            anyTableAppeared |= ipcspec::Init(); // IPC-only arrival also needs a new batch
        }
        anyTableAppeared |= pattern::ReloadModuleIfMissing("steamclient");

        if (anyTableAppeared && g_state.steamclientModule != nullptr) {
            AC_LOG_INFO(kModule, "Late pattern(s) became available; re-running hook batch.");
            InstallAllHooks(); // idempotent: only the previously-missed hooks
                               // register; LicenseManager::Init re-arms A2.
            status::Write();
        } else if (attempt == 1 || attempt % 6 == 0) {
            AC_LOG_INFO(kModule, "Late pattern retry #%d: steamclient=%d ipc=%d attached=%d.", attempt,
                        static_cast<int>(pattern::HasModule("steamclient")),
                        static_cast<int>(g_state.ipcSpec.loaded.load()),
                        static_cast<int>(g_state.steamclientModule != nullptr));
        }
        if (ReadyToInstallHooks()) {
            AC_LOG_INFO(kModule, "Steamclient attached and pattern tables available after "
                                 "retry; in-session hooks installed.");
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
    // Every hook in this batch resolves its target from the module handle; a null
    // handle would register hooks against nothing and report success.
    if (g_state.steamclientModule == nullptr) {
        AC_LOG_WARN(kModule, "Hook batch skipped: steamclient is not attached.");
        return;
    }
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
    // Idempotent re-run: registers only the previously missed hooks. The UI
    // batch is included because a late-arriving steamui table must install its
    // hooks in-session, exactly like the steamclient table does.
    InstallSteamClientBatch();
    InstallSteamUiBatch();
}

void InstallSteamUiBatch() {
    if (s_uiBatchInstalled.load(std::memory_order_acquire))
        return;
    std::lock_guard<std::mutex> batchLock(s_batchMutex);
    if (s_uiBatchInstalled.load(std::memory_order_acquire))
        return;
    if (g_state.steamuiModule == nullptr)
        return; // the attach worker keeps waiting; nothing to register yet
    if (!pattern::HasModule("steamui")) {
        // The late-pattern retry re-runs this function when the table appears;
        // the attach worker calls it again every tick, so log once.
        AC_LOG_WARN_ONCE(kModule, "SteamUI pattern table unavailable; app-overview hooks skipped.");
        return;
    }

    AC_LOG_INFO(kModule, "SteamUI hook batch: registering app-overview hooks.");
    // The batch stays retryable until the hook is really resolved: marking it
    // installed on an unresolved pattern would leave the layer off for the
    // session with no second attempt.
    if (!hooks::steamui::InstallAppOverviewHooks()) {
        AC_LOG_WARN_ONCE(kModule, "SteamUI app-overview hook unresolved; the batch stays retryable.");
        return;
    }
    if (g_state.hookManager.InstallAll()) {
        s_uiBatchInstalled.store(true, std::memory_order_release);
        AC_LOG_INFO(kModule, "SteamUI hooks enabled.");
    } else {
        AC_LOG_ERROR(kModule, "SteamUI hook enable failed.");
    }
    status::Write();
}

void StartSteamUiAttachRetry() {
    if (s_uiBatchInstalled.load(std::memory_order_acquire))
        return;
    bool expected = false;
    if (!s_uiAttachRetryStarted.compare_exchange_strong(expected, true))
        return;
    AC_LOG_INFO(kModule, "Starting steamui attach worker (%d ms poll, %d ms budget).", constants::kSteamuiAttachPollMs,
                constants::kSteamuiAttachBudgetMs);
    if (!workers::StartWorker("steamui_attach", [](std::atomic<bool>& stop) {
            const ULONGLONG start = GetTickCount64();
            ULONGLONG lastPatternRetry = 0;
            while (!stop.load(std::memory_order_relaxed)) {
                // The steamui table can be published after this session started
                // and no other worker reloads it (the late-pattern retry only
                // runs while the steamclient batch is incomplete). Retry it on
                // the same 10 s cadence, not on every 500 ms tick.
                if (!pattern::HasModule("steamui")) {
                    const ULONGLONG now = GetTickCount64();
                    if (lastPatternRetry == 0 ||
                        now - lastPatternRetry >= static_cast<ULONGLONG>(constants::kPatternLateRetryIntervalMs)) {
                        lastPatternRetry = now;
                        pattern::ReloadModuleIfMissing("steamui");
                    }
                }
                if (AttachSteamui(0)) {
                    status::Write();
                    InstallSteamUiBatch();
                    if (s_uiBatchInstalled.load(std::memory_order_acquire))
                        return;
                }
                if (GetTickCount64() - start >= static_cast<ULONGLONG>(constants::kSteamuiAttachBudgetMs)) {
                    AC_LOG_WARN(kModule,
                                "SteamUI not mapped after %d ms; app-overview hooks stay off for this session.",
                                constants::kSteamuiAttachBudgetMs);
                    status::Write();
                    return;
                }
                // Stop-aware sleep: shutdown must not wait for a full poll tick.
                for (int i = 0; i < 10 && !stop.load(std::memory_order_relaxed); ++i)
                    Sleep(50);
            }
        })) {
        s_uiAttachRetryStarted.store(false);
        AC_LOG_ERROR(kModule, "Could not start the steamui attach worker; "
                              "app-overview hooks need a Steam restart.");
    }
}

void StartPatternLateRetry() {
    if (ReadyToInstallHooks())
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
