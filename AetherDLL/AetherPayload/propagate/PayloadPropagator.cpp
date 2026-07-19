#include "pch.h"
#include "PayloadPropagator.h"

#include <MinHook.h>

#include <string>

#include "../log/PayloadLog.h"
#include "../util/RemoteDeploy.h"  // dichiarazione

namespace ac::payloadprop {
namespace {

// Payload DLL private path cache used by child-process propagation. The payload
// has no main-process AetherCoreState, so this is allowed per architecture docs.
wchar_t s_selfPath[MAX_PATH] = {};
using CreateProcessW_t = BOOL(WINAPI*)(LPCWSTR, LPWSTR, LPSECURITY_ATTRIBUTES,
    LPSECURITY_ATTRIBUTES, BOOL, DWORD, LPVOID, LPCWSTR, LPSTARTUPINFOW, LPPROCESS_INFORMATION);
using CreateProcessAsUserW_t = BOOL(WINAPI*)(HANDLE, LPCWSTR, LPWSTR,
    LPSECURITY_ATTRIBUTES, LPSECURITY_ATTRIBUTES, BOOL, DWORD, LPVOID,
    LPCWSTR, LPSTARTUPINFOW, LPPROCESS_INFORMATION);
CreateProcessW_t oCreateProcessW = nullptr;
CreateProcessAsUserW_t oCreateProcessAsUserW = nullptr;

BOOL Spawn(HANDLE token, LPCWSTR app, LPWSTR cmd, LPSECURITY_ATTRIBUTES pa,
           LPSECURITY_ATTRIBUTES ta, BOOL inherit, DWORD flags, LPVOID env,
           LPCWSTR cwd, LPSTARTUPINFOW si, LPPROCESS_INFORMATION pi) {
    const DWORD spawnFlags = flags | CREATE_SUSPENDED;
    BOOL ok = token
        ? oCreateProcessAsUserW(token, app, cmd, pa, ta, inherit, spawnFlags, env, cwd, si, pi)
        : oCreateProcessW(app, cmd, pa, ta, inherit, spawnFlags, env, cwd, si, pi);
    if (!ok) return ok;

    const bool injected = remoteinject::LoadDll(pi->hProcess, s_selfPath);
    payloadlog::Write("Payload process propagation pid=" + std::to_string(pi->dwProcessId) +
                      (injected ? " succeeded." : " FAILED."));
    if (!(flags & CREATE_SUSPENDED)) ResumeThread(pi->hThread);
    return ok;
}

BOOL WINAPI hkCreateProcessW(LPCWSTR app, LPWSTR cmd, LPSECURITY_ATTRIBUTES pa,
    LPSECURITY_ATTRIBUTES ta, BOOL inherit, DWORD flags, LPVOID env,
    LPCWSTR cwd, LPSTARTUPINFOW si, LPPROCESS_INFORMATION pi) {
    return Spawn(nullptr, app, cmd, pa, ta, inherit, flags, env, cwd, si, pi);
}

BOOL WINAPI hkCreateProcessAsUserW(HANDLE token, LPCWSTR app, LPWSTR cmd,
    LPSECURITY_ATTRIBUTES pa, LPSECURITY_ATTRIBUTES ta, BOOL inherit, DWORD flags,
    LPVOID env, LPCWSTR cwd, LPSTARTUPINFOW si, LPPROCESS_INFORMATION pi) {
    return Spawn(token, app, cmd, pa, ta, inherit, flags, env, cwd, si, pi);
}

}  // namespace

void Install(HMODULE self) {
    if (!GetModuleFileNameW(self, s_selfPath, MAX_PATH)) return;
    HMODULE k32 = GetModuleHandleW(L"kernel32.dll");
    if (!k32) return;
    oCreateProcessW = reinterpret_cast<CreateProcessW_t>(GetProcAddress(k32, "CreateProcessW"));
    oCreateProcessAsUserW = reinterpret_cast<CreateProcessAsUserW_t>(GetProcAddress(k32, "CreateProcessAsUserW"));
    const auto init = MH_Initialize();
    if (init != MH_OK && init != MH_ERROR_ALREADY_INITIALIZED) return;
    if (oCreateProcessW) MH_CreateHook(oCreateProcessW, hkCreateProcessW, reinterpret_cast<void**>(&oCreateProcessW));
    if (oCreateProcessAsUserW) MH_CreateHook(oCreateProcessAsUserW, hkCreateProcessAsUserW, reinterpret_cast<void**>(&oCreateProcessAsUserW));
    MH_EnableHook(MH_ALL_HOOKS);
    payloadlog::Write("Payload process propagation hooks installed.");
}

}  // namespace ac::payloadprop
