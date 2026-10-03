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
#include "hooks/steamui/SteamUIHook.h"
#include "hooks/wire/PacketRouter.h"
#include "utils/GameNameResolver.h"
#include "utils/IpcSpec.h"
#include "utils/PatternEngine.h"

namespace ac::bootstrap {
namespace {

namespace steamui = ac::hooks::steamui;

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

std::atomic<bool> s_steamUiRetryStarted{false};
std::atomic<bool> s_patternRetryStarted{false};
std::atomic<bool> s_patternRetryStop{false};

// Ritardo fra un tentativo e l'altro del retry steamui (5 × 100 ms = 500 ms).
constexpr int kSteamUiTicksPerCheck = 5;

bool PatternsFullyAvailable() {
    return pattern::HasModule("steamclient") &&
           pattern::HasModule("steamui") &&
           g_state.ipcSpec.loaded;
}

void SteamUiRetryWorker(std::atomic<bool>& stop) {
    int elapsedMs = 0;
    while (!stop.load(std::memory_order_relaxed) &&
           elapsedMs < constants::kSteamUiDeferredTimeoutMs) {
        if (steamui::InstallSteamUiRedirect()) {
            AC_LOG_INFO(kModule, "SteamUI redirect installed after %d ms (deferred retry).",
                        elapsedMs);
            return;
        }
        for (int i = 0; i < kSteamUiTicksPerCheck && !stop.load(std::memory_order_relaxed); ++i) {
            Sleep(constants::kSteamUiPollIntervalMs);
            elapsedMs += constants::kSteamUiPollIntervalMs;
        }
    }
    if (!stop.load(std::memory_order_relaxed)) {
        AC_LOG_WARN(kModule, "SteamUI deferred retry gave up after %d ms; "
                             "steamclient hooks remain installed, redirect absent.",
                    constants::kSteamUiDeferredTimeoutMs);
    }
}

void PatternLateRetryWorker(std::atomic<bool>& stop) {
    for (int attempt = 1; attempt <= constants::kPatternLateRetryMaxAttempts; ++attempt) {
        if (stop.load(std::memory_order_relaxed) || s_patternRetryStop.load(std::memory_order_relaxed)) return;
        if (PatternsFullyAvailable()) return;

        bool anyTableAppeared = false;
        if (!g_state.ipcSpec.loaded) {
            anyTableAppeared |= ipcspec::Init();  // IPC-only arrival also needs a new batch
        }
        anyTableAppeared |= pattern::ReloadModuleIfMissing("steamclient");
        anyTableAppeared |= pattern::ReloadModuleIfMissing("steamui");

        if (anyTableAppeared) {
            AC_LOG_INFO(kModule,
                        "Late pattern(s) became available; re-running hook batch.");
            InstallAllHooks();  // idempotent: only the previously-missed hooks
                                // register; LicenseManager::Init re-arms A2.
            status::Write();
        } else if (attempt == 1 || attempt % 6 == 0) {
            AC_LOG_INFO(kModule, "Late pattern retry #%d: steamclient=%d "
                                 "steamui=%d ipc=%d.",
                        attempt, static_cast<int>(pattern::HasModule("steamclient")),
                        static_cast<int>(pattern::HasModule("steamui")),
                        static_cast<int>(g_state.ipcSpec.loaded.load()));
        }
        if (PatternsFullyAvailable()) {
            AC_LOG_INFO(kModule, "All pattern tables available after retry; "
                                 "in-session hooks installed.");
            return;
        }

        // Sleep the interval in small steps so shutdown stays snappy.
        for (int elapsed = 0; elapsed < constants::kPatternLateRetryIntervalMs &&
                            !stop.load(std::memory_order_relaxed);
             elapsed += constants::kSteamUiPollIntervalMs) {
            Sleep(constants::kSteamUiPollIntervalMs);
        }
    }
    AC_LOG_WARN(kModule, "Late pattern retry gave up after %d attempt(s); "
                         "restart Steam once the patterns are published.",
                constants::kPatternLateRetryMaxAttempts);
}

}  // namespace

std::mutex& BatchMutex() { return s_batchMutex; }

void InstallSteamClientBatch() {
    std::lock_guard<std::mutex> batchLock(s_batchMutex);
    AC_LOG_INFO(kModule, "Hook batch: registering kernel32 + steamclient hooks.");
    // kernel32.dll hooks (pre-entry payload injection): install before any game
    // can be launched so the first SpawnProcess → CreateProcessW chain is covered.
    RegisterCreateProcessHooks();

    RegisterOwnershipHooks(g_state.diversionModule);
    RegisterDepotHooks(g_state.diversionModule);
    RegisterDecryptionKeyHook(g_state.diversionModule);
    RegisterAetherOnlineHooks(g_state.diversionModule);

    // IPC layer: capture helpers must resolve before the bus arms, and the bus
    // registers its command handlers internally.
    capture::Init(g_state.diversionModule);
    RegisterIpcBus(g_state.diversionModule);

    // Localized titles for presence (game_extra_info / PersonaState game_name).
    // Arms a one-shot capture on GetAppDataFromAppInfo; soft-fails if missing.
    gamename::Init(g_state.diversionModule);

    // Wire layer: outgoing/incoming packet manipulation (presence included).
    RegisterPacketRouter(g_state.diversionModule);

    // License/controller compatibility hooks.
    RegisterLicenseHooks(g_state.diversionModule);

    if (g_state.hookManager.InstallAll()) {
        g_state.hooksInstalled.store(true);
        AC_LOG_INFO(kModule, "Steamclient hooks enabled (batch 1).");
    } else {
        AC_LOG_ERROR(kModule, "Steamclient hook enable failed (batch 1).");
    }

    // Publish the state after the main batch; the steamui redirect (batch 2)
    // will republish when it installs.
    status::Write();
}

void ArmSteamUiRedirect() {
    if (steamui::InstallSteamUiRedirect()) return;   // già attivo (o live mode)
    bool expected = false;
    if (!s_steamUiRetryStarted.compare_exchange_strong(expected, true)) return;
    AC_LOG_INFO(kModule, "SteamUI redirect not ready yet; starting deferred "
                         "retry worker (timeout %d ms).",
                constants::kSteamUiDeferredTimeoutMs);
    if (!workers::StartWorker("steamui_redirect_retry", SteamUiRetryWorker)) {
        s_steamUiRetryStarted.store(false);
        AC_LOG_ERROR(kModule, "Could not start the SteamUI redirect retry worker.");
    }
}

void InstallAllHooks() {
    // Fase 2 prima: il redirect deve armarsi il prima possibile (finestra del
    // vecchio step-9), poi il batch steamclient. Riesecuzione idempotente.
    ArmSteamUiRedirect();
    InstallSteamClientBatch();
}

void StartPatternLateRetry() {
    if (PatternsFullyAvailable()) return;  // nothing to retry
    bool expected = false;
    if (!s_patternRetryStarted.compare_exchange_strong(expected, true)) return;
    s_patternRetryStop.store(false, std::memory_order_relaxed);
    AC_LOG_INFO(kModule, "Starting late-pattern retry worker (some pattern tables "
                         "were unavailable at init; %d attempts, %d ms apart).",
                constants::kPatternLateRetryMaxAttempts,
                constants::kPatternLateRetryIntervalMs);
    if (!workers::StartWorker("pattern_late_retry", PatternLateRetryWorker)) {
        s_patternRetryStarted.store(false);
        AC_LOG_ERROR(kModule, "Could not start the late-pattern retry worker; "
                              "missing hooks will need a Steam restart.");
    }
}

void RequestRetryStop() {
    s_patternRetryStop.store(true, std::memory_order_relaxed);
    // Il flag di stop del worker steamui è gestito dall'infrastruttura workers
    // (workers::Shutdown imposta tutti i flag e fa il join); qui chiediamo solo
    // esplicitamente lo stop del retry dei pattern, che ha un flag suo perché
    // può terminare anche "da solo" quando le tabelle diventano complete.
    AC_LOG_INFO(kModule, "Hook retry stop requested; workers::Shutdown performs the join.");
}

}  // namespace ac::bootstrap
