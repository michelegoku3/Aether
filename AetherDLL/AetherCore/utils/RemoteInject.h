#pragma once

#include <windows.h>

#include <cstdint>
#include <string>
#include <string_view>

#include "utils/Strings.h"

// ---------------------------------------------------------------------------
// RemoteInject — shared helpers for DLL injection into remote processes.
//
// Extracted from OnlinePayload.cpp during the CreateProcessHooks refactor
// (audit §3.4, 2026-07-12) so the pre-entry CreateProcess hook and the
// PipeWatch-triggered fallback both use the same injection primitives without
// duplication.
//
// Stateless.  No globals, no mutex.  Callers own the process HANDLE.
// ---------------------------------------------------------------------------
namespace ac::inject {

// Strict UTF-8 -> UTF-16 (shared implementation, utils/Strings.h).
// Invalid input produces an empty wstring: callers must check before passing
// the result to a path-consuming API.

// Returns true when the process identified by |process| is running under
// WOW64 (i.e. is a 32-bit process on a 64-bit OS).
// The AetherPayload.dll is x64-only for now, so 32-bit targets are
// skipped with a log message rather than injected with an incompatible DLL.
inline bool IsWow64Target(HANDLE process) {
    BOOL wow64 = FALSE;
    if (!IsWow64Process(process, &wow64)) return false;
    return wow64 == TRUE;
}

// Injects |dllPath| into the remote process using the classic
// VirtualAllocEx → WriteProcessMemory → CreateRemoteThread(LoadLibraryW)
// pattern.  Returns true on success (the remote LoadLibraryW returned).
// The remote thread is waited on for up to 5 seconds before this call
// returns — a hung loader won't block the caller indefinitely.
inline bool RemoteLoadDll(HANDLE process, const std::wstring& dllPath) {
    auto loadLib = reinterpret_cast<LPTHREAD_START_ROUTINE>(
        GetProcAddress(GetModuleHandleW(L"kernel32.dll"), "LoadLibraryW"));
    if (!loadLib) return false;

    const SIZE_T bytes = (dllPath.size() + 1) * sizeof(wchar_t);
    void* mem = VirtualAllocEx(process, nullptr, bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if (!mem) return false;

    bool ok = false;
    if (WriteProcessMemory(process, mem, dllPath.c_str(), bytes, nullptr)) {
        if (HANDLE t = CreateRemoteThread(process, nullptr, 0, loadLib, mem, 0, nullptr)) {
            ok = (WaitForSingleObject(t, 5000) == WAIT_OBJECT_0);
            CloseHandle(t);
        }
    }
    VirtualFreeEx(process, mem, 0, MEM_RELEASE);
    return ok;
}

}  // namespace ac::inject
