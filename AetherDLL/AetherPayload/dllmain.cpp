#include "pch.h"
#include <windows.h>
#include <psapi.h>

#include <string>

#include "eos/EpicOnlineBridge.h"
#include "log/PayloadLog.h"
#include "propagate/PayloadPropagator.h"

namespace {

void TryInstallIfPresent() {
    if (HMODULE eos = GetModuleHandleW(L"EOSSDK-Win64-Shipping.dll")) {
        ac::eosbridge::InstallOn(eos);
    }
}

DWORD WINAPI PayloadMain(LPVOID self) {
    ac::payloadlog::Init(static_cast<HMODULE>(self));
    ac::payloadlog::Write("Payload attached to game process.");
    ac::payloadprop::Install(static_cast<HMODULE>(self));

    for (int i = 0; i < 120; ++i) {
        TryInstallIfPresent();
        if (GetModuleHandleW(L"EOSSDK-Win64-Shipping.dll")) break;
        Sleep(500);
    }
    return 0;
}

}  // namespace

BOOL APIENTRY DllMain(HMODULE hModule, DWORD reason, LPVOID) {
    if (reason == DLL_PROCESS_ATTACH) {
        DisableThreadLibraryCalls(hModule);
        if (HANDLE h = CreateThread(nullptr, 0, PayloadMain, hModule, 0, nullptr)) {
            CloseHandle(h);
        }
    }
    return TRUE;
}
