#include "pch.h"
#include "hooks/steamui/SteamUIHook.h"

#include <atomic>
#include <cstring>
#include <mutex>
#include <thread>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "hooks/aetheronline/CreateProcessHooks.h"
#include "hooks/steamclient/DecryptionKeyHook.h"
#include "hooks/steamclient/DepotHooks.h"
#include "core/HookManager.h"
#include "hooks/ipc/IPCBus.h"
#include "hooks/steamclient/LicenseHooks.h"
#include "core/Logger.h"
#include "hooks/steamclient/AetherOnlineHooks.h"
#include "hooks/steamclient/OwnershipHooks.h"
#include "hooks/wire/PacketRouter.h"
#include "utils/GameNameResolver.h"
#include "utils/IpcSpec.h"
#include "utils/PatternEngine.h"
#include "hooks/ipc/SteamCapture.h"
#include "diagnostics/StatusWriter.h"

namespace ac::hooks {
namespace {

constexpr const char* kModule = "SteamUI";

// steamui.dll!LoadModuleWithPath loads the live steamclient64.dll; we redirect
// it to our diverted, hookable copy. This hook requires steamui.dll to be
// mapped, so it is installed in a deferred second batch (see below).
using LoadModuleWithPath_t = HMODULE (*)(const char*, bool);
LoadModuleWithPath_t o_LoadModuleWithPath = nullptr;

// ---- Deferred steamui redirect retry (A7) ----------------------------------
// Installs the LoadModuleWithPath redirect as soon as steamui.dll appears,
// without blocking the steamclient hook install. Module plumbing (private
// lifecycle), so the control block lives here, not in AetherCoreState.
std::thread s_retryThread;
std::atomic<bool> s_retryStop{false};
std::atomic<bool> s_retryStarted{false};

// Serialises the hook batches. The batch can be re-run in-session by the
// late-pattern retry thread while the deferred steamui retry thread may also
// call InstallSteamUiRedirect — HookManager is not internally synchronised,
// so both paths take this lock.
std::mutex s_batchMutex;

HMODULE h_LoadModuleWithPath(const char* path, bool flags) {
    if (path && std::strstr(path, "steamclient64.dll")) {
        AC_LOG_INFO_ONCE(kModule, "Redirecting steamclient64.dll load to acoverlay.dll.");
        return g_state.diversionModule;
    }
    return o_LoadModuleWithPath(path, flags);
}

// Installs the steamui redirect if steamui.dll is mapped. Returns true when the
// redirect was registered+enabled (or already present); false when steamui is
// not available yet (caller decides whether to retry).
bool InstallSteamUiRedirect() {
    std::lock_guard<std::mutex> batchLock(s_batchMutex);
    HMODULE steamui = GetModuleHandleA("steamui.dll");
    if (!steamui) return false;
    g_state.steamuiModule = steamui;

    if (void* addr = pattern::ResolveAddress("LoadModuleWithPath", "steamui", steamui)) {
        g_state.hookManager.RegisterHook("LoadModuleWithPath", addr,
                                   reinterpret_cast<void**>(&o_LoadModuleWithPath),
                                   reinterpret_cast<void*>(h_LoadModuleWithPath));
        if (g_state.hookManager.InstallAll()) {
            AC_LOG_INFO(kModule, "SteamUI redirect installed.");
        } else {
            AC_LOG_ERROR(kModule, "SteamUI redirect enable failed.");
        }
    } else {
        g_state.hookManager.RecordMissed("LoadModuleWithPath");
        AC_LOG_WARN(kModule, "LoadModuleWithPath unresolved; steamclient hooks may not apply.");
    }

    // Republish the final hook state now that the redirect is (or is not) in.
    status::Write();
    return true;
}

void SteamUiRetryThread() {
    constexpr int kTicksPerCheck = 5;  // 5 × 100ms = 500ms per check (cheap)
    int elapsedMs = 0;
    while (!s_retryStop.load(std::memory_order_relaxed) &&
           elapsedMs < constants::kSteamUiDeferredTimeoutMs) {
        if (InstallSteamUiRedirect()) {
            AC_LOG_INFO(kModule, "SteamUI redirect installed after %d ms.", elapsedMs);
            return;
        }
        for (int i = 0; i < kTicksPerCheck && !s_retryStop.load(std::memory_order_relaxed); ++i) {
            Sleep(constants::kSteamUiPollIntervalMs);
            elapsedMs += constants::kSteamUiPollIntervalMs;
        }
    }
    if (!s_retryStop.load(std::memory_order_relaxed)) {
        AC_LOG_WARN(kModule, "SteamUI deferred retry gave up after %d ms; "
                             "steamclient hooks remain installed, redirect absent.",
                    constants::kSteamUiDeferredTimeoutMs);
    }
}

void StartSteamUiRetry() {
    bool expected = false;
    if (!s_retryStarted.compare_exchange_strong(expected, true)) return;
    s_retryStop.store(false, std::memory_order_relaxed);
    s_retryThread = std::thread(SteamUiRetryThread);
}

void StopSteamUiRetry() {
    s_retryStop.store(true, std::memory_order_relaxed);
    if (s_retryThread.joinable()) s_retryThread.join();
}

// Registers and enables every steamclient (and kernel32) hook in one atomic
// batch. Does NOT depend on steamui.dll. Idempotent when re-run: HookManager
// de-duplicates registrations by name, so a second run only queues the hooks
// that were previously missed (e.g. pattern table arrived late).
void InstallSteamClientBatch() {
    std::lock_guard<std::mutex> batchLock(s_batchMutex);
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

}  // namespace

void InstallAllHooks() {
    // 1. Try the steamui redirect immediately (common case: steamui.dll is
    //    already mapped at init). This registers+enables only the redirect in
    //    its own batch, so it is active as early as possible.
    const bool redirectReady = InstallSteamUiRedirect();

    // 2. Install all steamclient hooks immediately (no steamui dependency).
    //    The redirect batch (if any) and this batch accumulate in HookManager.
    InstallSteamClientBatch();

    // 3. If steamui.dll was not present yet, retry the redirect in the
    //    background with a bounded budget.
    if (!redirectReady) {
        StartSteamUiRetry();
    }
}

void ShutdownSteamUiRetry() {
    StopSteamUiRetry();
}

// ---------------------------------------------------------------------------
// Late pattern availability retry
//
// If a module's pattern table was missing at init (fresh Steam build whose
// patterns were not published yet, or a slow/offline start), the affected
// hooks stay skipped for the whole session and a restart is needed once the
// patterns appear. This background thread re-probes the sources for a bounded
// window; the moment a table appears it re-runs the (idempotent) hook batch,
// so the missing hooks install IN-SESSION, and the LicenseManager::Init that
// runs inside the batch re-arms the package-0 startup retry so ownership
// top-up can still happen after the one-shot LoadPackage window was missed.
// ---------------------------------------------------------------------------
namespace {

std::thread s_patternRetryThread;
std::atomic<bool> s_patternRetryStop{false};
std::atomic<bool> s_patternRetryStarted{false};

bool PatternsFullyAvailable() {
    return !g_state.patterns.steamclient.empty() &&
           !g_state.patterns.steamui.empty() &&
           g_state.ipcSpec.loaded;
}

void PatternLateRetryThread() {
    for (int attempt = 1; attempt <= constants::kPatternLateRetryMaxAttempts; ++attempt) {
        if (s_patternRetryStop.load(std::memory_order_relaxed)) return;
        if (PatternsFullyAvailable()) return;

        bool anyTableAppeared = false;
        if (!g_state.ipcSpec.loaded) {
            ipcspec::Init();  // no-op once loaded; loads cache or downloads
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
                        attempt, static_cast<int>(!g_state.patterns.steamclient.empty()),
                        static_cast<int>(!g_state.patterns.steamui.empty()),
                        static_cast<int>(g_state.ipcSpec.loaded));
        }
        if (PatternsFullyAvailable()) {
            AC_LOG_INFO(kModule, "All pattern tables available after retry; "
                                 "in-session hooks installed.");
            return;
        }

        // Sleep the interval in small steps so shutdown stays snappy.
        for (int elapsed = 0; elapsed < constants::kPatternLateRetryIntervalMs &&
                            !s_patternRetryStop.load(std::memory_order_relaxed);
             elapsed += constants::kSteamUiPollIntervalMs) {
            Sleep(constants::kSteamUiPollIntervalMs);
        }
    }
    AC_LOG_WARN(kModule, "Late pattern retry gave up after %d attempt(s); "
                         "restart Steam once the patterns are published.",
                constants::kPatternLateRetryMaxAttempts);
}

}  // namespace

void StartPatternLateRetry() {
    if (PatternsFullyAvailable()) return;  // nothing to retry
    bool expected = false;
    if (!s_patternRetryStarted.compare_exchange_strong(expected, true)) return;
    s_patternRetryStop.store(false, std::memory_order_relaxed);
    AC_LOG_INFO(kModule, "Starting late-pattern retry (some pattern tables "
                         "were unavailable at init; %d attempts, %d ms apart).",
                constants::kPatternLateRetryMaxAttempts,
                constants::kPatternLateRetryIntervalMs);
    s_patternRetryThread = std::thread(PatternLateRetryThread);
}

void StopPatternLateRetry() {
    s_patternRetryStop.store(true, std::memory_order_relaxed);
    if (s_patternRetryThread.joinable()) s_patternRetryThread.join();
}

}  // namespace ac::hooks
