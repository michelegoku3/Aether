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
#include "utils/Strings.h"

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

// Gap between steamui retry attempts (500 ms backstop quantum).
constexpr int kSteamUiCheckIntervalMs = 500;

// Minimal LdrRegisterDllNotification plumbing (ntdll, always present on the
// Win10+ target): wakes the steamui retry the moment ANY dll loads instead
// of polling blindly. The callback runs under the loader lock, so it only
// matches the filename and sets an event: no locks, no allocations, no I/O.
struct LdrLoadedData {
    ULONG flags;
    const wchar_t* fullDllName;
    const wchar_t* baseDllName;
    void* dllBase;
    ULONG sizeOfImage;
};
using LdrNotifyFn = void(CALLBACK*)(ULONG reason, LdrLoadedData* data, void* context);
using LdrRegisterFn = LONG(NTAPI*)(ULONG flags, LdrNotifyFn callback, void* context, void** cookie);
using LdrUnregisterFn = LONG(NTAPI*)(void* cookie);
constexpr ULONG kLdrReasonLoaded = 1;

bool EndsWithIgnoreCase(const wchar_t* str, const wchar_t* suffix) {
    if (!str || !suffix)
        return false;
    std::size_t n = 0, m = 0;
    while (str[n] != L'\0')
        ++n;
    while (suffix[m] != L'\0')
        ++m;
    if (m > n)
        return false;
    for (std::size_t i = 0; i < m; ++i) {
        wchar_t a = str[n - m + i], b = suffix[i];
        if (a >= L'A' && a <= L'Z')
            a = static_cast<wchar_t>(a + (L'a' - L'A'));
        if (b >= L'A' && b <= L'Z')
            b = static_cast<wchar_t>(b + (L'a' - L'A'));
        if (a != b)
            return false;
    }
    return true;
}

void CALLBACK OnAnyDllLoaded(ULONG reason, LdrLoadedData* data, void* context) {
    // Loader-lock context: event-set only.
    if (reason == kLdrReasonLoaded && data && context && EndsWithIgnoreCase(data->baseDllName, L"steamui.dll")) {
        SetEvent(static_cast<HANDLE>(context));
    }
}

bool PatternsFullyAvailable() {
    return pattern::HasModule("steamclient") && pattern::HasModule("steamui") && g_state.ipcSpec.loaded;
}

void SteamUiRetryWorker(std::atomic<bool>& stop) {
    // Event-driven wait: a DLL load wakes us early; the 500 ms quantum is
    // only a backstop (same check cadence as the old 5x100 ms polling).
    // Falls back to plain polling when the Ldr API is unavailable.
    HANDLE wake = CreateEventW(nullptr, FALSE, FALSE, nullptr);
    void* cookie = nullptr;
    LdrUnregisterFn unregister = nullptr;
    if (HMODULE ntdll = GetModuleHandleW(L"ntdll.dll")) {
        if (auto reg = reinterpret_cast<LdrRegisterFn>(GetProcAddress(ntdll, "LdrRegisterDllNotification"))) {
            unregister = reinterpret_cast<LdrUnregisterFn>(GetProcAddress(ntdll, "LdrUnregisterDllNotification"));
            if (reg(0, OnAnyDllLoaded, wake, &cookie) != 0)
                cookie = nullptr;
        }
    }
    int elapsedMs = 0;
    while (!stop.load(std::memory_order_relaxed) && elapsedMs < constants::kSteamUiDeferredTimeoutMs) {
        if (steamui::InstallSteamUiRedirect()) {
            AC_LOG_INFO(kModule, "SteamUI redirect installed after %d ms (deferred retry).", elapsedMs);
            break;
        }
        if (wake && cookie) {
            WaitForSingleObject(wake, static_cast<DWORD>(kSteamUiCheckIntervalMs));
        } else {
            Sleep(static_cast<DWORD>(kSteamUiCheckIntervalMs));
        }
        if (stop.load(std::memory_order_relaxed))
            break;
        elapsedMs += kSteamUiCheckIntervalMs;
    }
    if (cookie && unregister)
        unregister(cookie);
    if (wake)
        CloseHandle(wake);
    if (!stop.load(std::memory_order_relaxed) && elapsedMs >= constants::kSteamUiDeferredTimeoutMs) {
        AC_LOG_WARN(kModule,
                    "SteamUI deferred retry gave up after %d ms; "
                    "steamclient hooks remain installed, redirect absent.",
                    constants::kSteamUiDeferredTimeoutMs);
    }
}

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
        anyTableAppeared |= pattern::ReloadModuleIfMissing("steamui");

        if (anyTableAppeared) {
            AC_LOG_INFO(kModule, "Late pattern(s) became available; re-running hook batch.");
            InstallAllHooks(); // idempotent: only the previously-missed hooks
                               // register; LicenseManager::Init re-arms A2.
            status::Write();
        } else if (attempt == 1 || attempt % 6 == 0) {
            AC_LOG_INFO(kModule,
                        "Late pattern retry #%d: steamclient=%d "
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

        // Event-driven wait on the pattern directory: a freshly published
        // table wakes the next attempt at once instead of at the next 10 s
        // tick. The 250 ms quantum keeps shutdown snappy (was 100 ms: 2.5x
        // fewer idle wakeups for a bounded 150 ms of extra join latency).
        HANDLE change = FindFirstChangeNotificationW(ac::strings::Widen(g_state.patternDir).c_str(), FALSE,
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
    if (steamui::InstallSteamUiRedirect())
        return; // already armed (or live mode)
    bool expected = false;
    if (!s_steamUiRetryStarted.compare_exchange_strong(expected, true))
        return;
    AC_LOG_INFO(kModule,
                "SteamUI redirect not ready yet; starting deferred "
                "retry worker (timeout %d ms).",
                constants::kSteamUiDeferredTimeoutMs);
    if (!workers::StartWorker("steamui_redirect_retry", SteamUiRetryWorker)) {
        s_steamUiRetryStarted.store(false);
        AC_LOG_ERROR(kModule, "Could not start the SteamUI redirect retry worker.");
    }
}

void InstallAllHooks() {
    // Phase 2 first: the redirect must arm ASAP (old step-9 window), then the
    // steamclient batch. Idempotent re-run.
    ArmSteamUiRedirect();
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
    // The steamui worker's stop flag is owned by the workers infrastructure
    // (workers::Shutdown sets every flag and joins); here we only explicitly
    // stop the pattern retry, which has its own flag because it may also end
    // "on its own" when the tables become complete.
    AC_LOG_INFO(kModule, "Hook retry stop requested; workers::Shutdown performs the join.");
}

} // namespace ac::bootstrap
