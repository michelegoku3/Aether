#include "pch.h"
#include "PayloadPropagator.h"
#include "PropagationPolicy.h"

#include <MinHook.h>

#include <string>

#include "../log/PayloadLog.h"
#include "../util/RemoteDeploy.h"

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

// Logging must never prevent a newly created child from being resumed.
void LogDecision(DWORD pid, const std::wstring& image, const char* outcome,
                 const char* reason, DWORD error) noexcept {
    try {
        const int n = WideCharToMultiByte(CP_UTF8, 0, image.data(),
                                          static_cast<int>(image.size()), nullptr, 0, nullptr, nullptr);
        std::string utf8(n > 0 ? n : 0, '\0');
        if (n > 0) WideCharToMultiByte(CP_UTF8, 0, image.data(), static_cast<int>(image.size()),
                                      utf8.data(), n, nullptr, nullptr);
        payloadlog::Write("propagation pid=" + std::to_string(pid) + " exe=\"" + utf8 +
                          "\" outcome=" + outcome + " reason=" + reason +
                          " last_error_hint=" + std::to_string(error));
    } catch (...) { /* best-effort diagnostics, never affect child lifetime */ }
}

BOOL Spawn(bool asUser, HANDLE token, LPCWSTR app, LPWSTR cmd, LPSECURITY_ATTRIBUTES pa,
           LPSECURITY_ATTRIBUTES ta, BOOL inherit, DWORD flags, LPVOID env,
           LPCWSTR cwd, LPSTARTUPINFOW si, LPPROCESS_INFORMATION pi) {
    const DWORD spawnFlags = flags | CREATE_SUSPENDED;
    BOOL ok = asUser
        ? oCreateProcessAsUserW(token, app, cmd, pa, ta, inherit, spawnFlags, env, cwd, si, pi)
        : oCreateProcessW(app, cmd, pa, ta, inherit, spawnFlags, env, cwd, si, pi);
    const DWORD originalError = GetLastError();
    if (!ok) {
        LogDecision(0, L"", "spawn_failed", "create_process", originalError);
        SetLastError(originalError);
        return ok;
    }

    std::wstring image;
    try {
        // Inspect the actual created image, not a guessed/quoted command line.
        image.resize(32768);
        DWORD size = static_cast<DWORD>(image.size());
        wchar_t windowsDir[MAX_PATH] = {};
        const UINT rootSize = GetWindowsDirectoryW(windowsDir, MAX_PATH);
        if (!QueryFullProcessImageNameW(pi->hProcess, 0, image.data(), &size)) {
            const DWORD error = GetLastError();
            image.clear();
            LogDecision(pi->dwProcessId, image, "skipped", "identity_unavailable", error);
        } else {
            image.resize(size);
            const char* reason = (flags & CREATE_PROTECTED_PROCESS)
                ? "protected_process_flag"
                : BlockedReason(image, rootSize > 0 && rootSize < MAX_PATH ? windowsDir : L"");
            if (reason) {
                LogDecision(pi->dwProcessId, image, "skipped", reason, 0);
            } else {
                const bool injected = remoteinject::LoadDll(pi->hProcess, s_selfPath);
                const DWORD error = injected ? ERROR_SUCCESS : GetLastError();
                LogDecision(pi->dwProcessId, image, injected ? "helper_ok" : "helper_failed",
                            "eligible_child", error);
            }
        }
    } catch (...) {
        LogDecision(pi->dwProcessId, image, "skipped", "policy_exception", 0);
    }
    // Preserve caller-owned suspension even on policy/query/injection failures.
    if (!(flags & CREATE_SUSPENDED) && ResumeThread(pi->hThread) == static_cast<DWORD>(-1)) {
        LogDecision(pi->dwProcessId, image, "resume_failed", "resume_thread", GetLastError());
    }
    SetLastError(originalError);
    return ok;
}

BOOL WINAPI hkCreateProcessW(LPCWSTR app, LPWSTR cmd, LPSECURITY_ATTRIBUTES pa,
    LPSECURITY_ATTRIBUTES ta, BOOL inherit, DWORD flags, LPVOID env,
    LPCWSTR cwd, LPSTARTUPINFOW si, LPPROCESS_INFORMATION pi) {
    return Spawn(false, nullptr, app, cmd, pa, ta, inherit, flags, env, cwd, si, pi);
}

BOOL WINAPI hkCreateProcessAsUserW(HANDLE token, LPCWSTR app, LPWSTR cmd,
    LPSECURITY_ATTRIBUTES pa, LPSECURITY_ATTRIBUTES ta, BOOL inherit, DWORD flags,
    LPVOID env, LPCWSTR cwd, LPSTARTUPINFOW si, LPPROCESS_INFORMATION pi) {
    return Spawn(true, token, app, cmd, pa, ta, inherit, flags, env, cwd, si, pi);
}

}  // namespace

void Install(HMODULE self) {
    const DWORD pathSize = GetModuleFileNameW(self, s_selfPath, MAX_PATH);
    if (pathSize == 0 || pathSize >= MAX_PATH) {
        payloadlog::Write("Propagation disabled: payload path unavailable/truncated.");
        return;
    }
    HMODULE k32 = GetModuleHandleW(L"kernel32.dll");
    if (!k32) return;
    oCreateProcessW = reinterpret_cast<CreateProcessW_t>(GetProcAddress(k32, "CreateProcessW"));
    oCreateProcessAsUserW = reinterpret_cast<CreateProcessAsUserW_t>(GetProcAddress(k32, "CreateProcessAsUserW"));
    const auto init = MH_Initialize();
    if (init != MH_OK && init != MH_ERROR_ALREADY_INITIALIZED) {
        payloadlog::Write(std::string("Propagation initialization failed: ") + MH_StatusToString(init));
        return;
    }
    const auto create = oCreateProcessW
        ? MH_CreateHook(reinterpret_cast<void*>(oCreateProcessW), reinterpret_cast<void*>(hkCreateProcessW),
                        reinterpret_cast<void**>(&oCreateProcessW))
        : MH_ERROR_FUNCTION_NOT_FOUND;
    const auto asUser = oCreateProcessAsUserW
        ? MH_CreateHook(reinterpret_cast<void*>(oCreateProcessAsUserW), reinterpret_cast<void*>(hkCreateProcessAsUserW),
                        reinterpret_cast<void**>(&oCreateProcessAsUserW))
        : MH_ERROR_FUNCTION_NOT_FOUND;
    const auto enable = MH_EnableHook(MH_ALL_HOOKS);
    payloadlog::Write(std::string("Propagation hook results: CreateProcessW=") + MH_StatusToString(create) +
                      " CreateProcessAsUserW=" + MH_StatusToString(asUser) +
                      " enable=" + MH_StatusToString(enable) +
                      "; conservative filter: system/protected/crash/browser; unknown identity is skipped.");
}

}  // namespace ac::payloadprop
