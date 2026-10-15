#include "pch.h"
#include "RemoteDeploy.h"

#include <windows.h>

namespace ac::remoteinject {

bool LoadDll(HANDLE process, LPCWSTR dllPath) {
    auto loadLib =
        reinterpret_cast<LPTHREAD_START_ROUTINE>(GetProcAddress(GetModuleHandleW(L"kernel32.dll"), "LoadLibraryW"));
    if (!loadLib || !dllPath || dllPath[0] == L'\0')
        return false;

    const SIZE_T bytes = (wcslen(dllPath) + 1) * sizeof(wchar_t);
    void* mem = VirtualAllocEx(process, nullptr, bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if (!mem)
        return false;

    bool ok = false;
    if (WriteProcessMemory(process, mem, dllPath, bytes, nullptr)) {
        if (HANDLE t = CreateRemoteThread(process, nullptr, 0, loadLib, mem, 0, nullptr)) {
            // A thread that merely exited is not success: the remote
            // LoadLibraryW must have returned a non-null module handle.
            const bool waited = (WaitForSingleObject(t, 5000) == WAIT_OBJECT_0);
            DWORD code = 0;
            ok = waited && GetExitCodeThread(t, &code) && code != 0;
            CloseHandle(t);
        }
    }
    VirtualFreeEx(process, mem, 0, MEM_RELEASE);
    return ok;
}

} // namespace ac::remoteinject
