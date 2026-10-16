#include "pch.h"
#include <psapi.h>
#include <windows.h>

#include <string>

#include "core/Constants.h"
#include "eos/EpicOnlineBridge.h"
#include "log/PayloadLog.h"
#include "propagate/PayloadPropagator.h"

namespace {

// Set on DETACH so the worker wakes immediately instead of sleeping out the
// full EOS poll budget. Plain event (not atomic): created before the thread,
// signaled on detach, never waited on under the loader lock.
HANDLE g_stopEvent = nullptr;

void TryInstallIfPresent() {
    if (HMODULE eos = GetModuleHandleW(L"EOSSDK-Win64-Shipping.dll")) {
        ac::eosbridge::InstallOn(eos);
    }
}

DWORD WINAPI PayloadMain(LPVOID self) {
    ac::payloadlog::Init(static_cast<HMODULE>(self));
    ac::payloadlog::Write("Payload attached to game process.");
    ac::payloadprop::Install(static_cast<HMODULE>(self));

    // Same 60 s budget as always, but event-driven: detach wakes the wait at
    // once instead of sleeping out the remainder. Budget lives in
    // core/Constants.h (single tunable file).
    // NOTE: this thread intentionally outlives DllMain; it never touches the
    // loader lock (no LoadLibrary here: only GetModuleHandle polling).
    for (int i = 0; i < ac::constants::kPayloadEosPollAttempts; ++i) {
        TryInstallIfPresent();
        if (GetModuleHandleW(L"EOSSDK-Win64-Shipping.dll"))
            break;
        if (g_stopEvent && WaitForSingleObject(g_stopEvent, ac::constants::kPayloadEosPollIntervalMs) != WAIT_TIMEOUT)
            break;
    }
    return 0;
}

} // namespace

BOOL APIENTRY DllMain(HMODULE hModule, DWORD reason, LPVOID) {
    if (reason == DLL_PROCESS_ATTACH) {
        DisableThreadLibraryCalls(hModule);
        // DllMain itself does no I/O and takes no locks: it only records the
        // stop event and hands off to the worker. The event is manual-reset
        // and initially non-signaled; PayloadMain is the only waiter.
        g_stopEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        if (HANDLE h = CreateThread(nullptr, 0, PayloadMain, hModule, 0, nullptr)) {
            CloseHandle(h);
        }
    } else if (reason == DLL_PROCESS_DETACH) {
        // Loader lock: signal only, never wait. The worker observes the event
        // at its next 500 ms quantum and exits on its own.
        if (g_stopEvent)
            SetEvent(g_stopEvent);
    }
    return TRUE;
}
