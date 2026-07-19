#include "pch.h"
#include "hooks/onlinefix/CreateProcessHooks.h"

#include <windows.h>

#include <string>

#include "core/AetherCoreState.h"
#include "core/HookManager.h"
#include "core/Logger.h"
#include "utils/RemoteInject.h"

namespace ac::hooks {
namespace {

constexpr const char* kModule = "CreateProcess";

// kernel32.dll trampolines
using CreateProcessW_t = BOOL(WINAPI*)(LPCWSTR, LPWSTR, LPSECURITY_ATTRIBUTES,
                                       LPSECURITY_ATTRIBUTES, BOOL, DWORD,
                                       LPVOID, LPCWSTR, LPSTARTUPINFOW,
                                       LPPROCESS_INFORMATION);
using CreateProcessAsUserW_t = BOOL(WINAPI*)(HANDLE, LPCWSTR, LPWSTR,
                                             LPSECURITY_ATTRIBUTES,
                                             LPSECURITY_ATTRIBUTES, BOOL, DWORD,
                                             LPVOID, LPCWSTR, LPSTARTUPINFOW,
                                             LPPROCESS_INFORMATION);

CreateProcessW_t       o_CreateProcessW       = nullptr;
CreateProcessAsUserW_t o_CreateProcessAsUserW = nullptr;

// ---------------------------------------------------------------------------
// Injection helper — called after the child is created suspended.
// ---------------------------------------------------------------------------
void InjectIntoChild(HANDLE hProcess, DWORD pid) {
    if (GetFileAttributesA(g_state.payloadDllPath.c_str()) == INVALID_FILE_ATTRIBUTES) {
        // Once per game session (logger dedup) instead of a module-local atomic.
        AC_LOG_WARN_ONCE(kModule, "Payload DLL missing: %s", g_state.payloadDllPath.c_str());
        return;
    }

    // PID dedup: if PipeWatch already injected (unlikely this early, but
    // defensive), skip.
    {
        std::lock_guard<std::mutex> lock(g_state.onlinePayload.mutex);
        if (g_state.onlinePayload.injectedPids.count(pid)) return;
        g_state.onlinePayload.injectedPids.insert(pid);
    }

    // x86 targets are skipped because the payload is x64-only for now.
    if (inject::IsWow64Target(hProcess)) {
        ++g_state.onlinePayload.injectFailureCount;
        AC_LOG_WARN(kModule, "Skipping x86 target pid=%u (payload is x64 only).", pid);
        return;
    }

    const std::wstring path = inject::Widen(g_state.payloadDllPath);
    if (path.empty()) {
        ++g_state.onlinePayload.injectFailureCount;
        AC_LOG_WARN(kModule, "Payload path conversion failed pid=%u.", pid);
        return;
    }

    if (inject::RemoteLoadDll(hProcess, path)) {
        ++g_state.onlinePayload.injectSuccessCount;
        AC_LOG_INFO(kModule, "Pre-entry payload injected pid=%u.", pid);
    } else {
        ++g_state.onlinePayload.injectFailureCount;
        AC_LOG_WARN(kModule, "Pre-entry payload injection failed pid=%u.", pid);
    }
}

// ---------------------------------------------------------------------------
// Hook bodies
// ---------------------------------------------------------------------------

BOOL WINAPI h_CreateProcessW(LPCWSTR appName, LPWSTR cmdLine,
                             LPSECURITY_ATTRIBUTES processAttrs,
                             LPSECURITY_ATTRIBUTES threadAttrs,
                             BOOL inheritHandles, DWORD creationFlags,
                             LPVOID environment, LPCWSTR currentDir,
                             LPSTARTUPINFOW startupInfo,
                             LPPROCESS_INFORMATION processInfo) {
    steam::AppId appId = g_state.onlineFixRealAppId.load();
    if (appId == 0) {
        return o_CreateProcessW(appName, cmdLine, processAttrs, threadAttrs,
                                inheritHandles, creationFlags, environment,
                                currentDir, startupInfo, processInfo);
    }

    const DWORD originalFlags = creationFlags;
    const BOOL result = o_CreateProcessW(appName, cmdLine, processAttrs, threadAttrs,
                                         inheritHandles, creationFlags | CREATE_SUSPENDED,
                                         environment, currentDir, startupInfo, processInfo);
    if (!result) return result;

    InjectIntoChild(processInfo->hProcess, processInfo->dwProcessId);

    if (!(originalFlags & CREATE_SUSPENDED)) {
        ResumeThread(processInfo->hThread);
    }
    return result;
}

BOOL WINAPI h_CreateProcessAsUserW(HANDLE token, LPCWSTR appName, LPWSTR cmdLine,
                                   LPSECURITY_ATTRIBUTES processAttrs,
                                   LPSECURITY_ATTRIBUTES threadAttrs,
                                   BOOL inheritHandles, DWORD creationFlags,
                                   LPVOID environment, LPCWSTR currentDir,
                                   LPSTARTUPINFOW startupInfo,
                                   LPPROCESS_INFORMATION processInfo) {
    steam::AppId appId = g_state.onlineFixRealAppId.load();
    if (appId == 0) {
        return o_CreateProcessAsUserW(token, appName, cmdLine, processAttrs, threadAttrs,
                                      inheritHandles, creationFlags, environment,
                                      currentDir, startupInfo, processInfo);
    }

    const DWORD originalFlags = creationFlags;
    const BOOL result = o_CreateProcessAsUserW(token, appName, cmdLine, processAttrs,
                                               threadAttrs, inheritHandles,
                                               creationFlags | CREATE_SUSPENDED,
                                               environment, currentDir,
                                               startupInfo, processInfo);
    if (!result) return result;

    InjectIntoChild(processInfo->hProcess, processInfo->dwProcessId);

    if (!(originalFlags & CREATE_SUSPENDED)) {
        ResumeThread(processInfo->hThread);
    }
    return result;
}

}  // namespace

void RegisterCreateProcessHooks() {
    HMODULE k32 = GetModuleHandleW(L"kernel32.dll");
    if (!k32) {
        AC_LOG_ERROR(kModule, "kernel32.dll not loaded; cannot install CreateProcess hooks.");
        return;
    }

    void* cpw = GetProcAddress(k32, "CreateProcessW");
    if (cpw) {
        g_state.hookManager.RegisterHook("CreateProcessW", cpw,
                                         reinterpret_cast<void**>(&o_CreateProcessW),
                                         reinterpret_cast<void*>(h_CreateProcessW));
    } else {
        g_state.hookManager.RecordMissed("CreateProcessW");
        AC_LOG_ERROR(kModule, "CreateProcessW not found in kernel32.dll.");
    }

    void* cpau = GetProcAddress(k32, "CreateProcessAsUserW");
    if (cpau) {
        g_state.hookManager.RegisterHook("CreateProcessAsUserW", cpau,
                                         reinterpret_cast<void**>(&o_CreateProcessAsUserW),
                                         reinterpret_cast<void*>(h_CreateProcessAsUserW));
    } else {
        g_state.hookManager.RecordMissed("CreateProcessAsUserW");
        AC_LOG_ERROR(kModule, "CreateProcessAsUserW not found in kernel32.dll.");
    }

    AC_LOG_INFO(kModule, "CreateProcess hooks registered.");
}

}  // namespace ac::hooks
