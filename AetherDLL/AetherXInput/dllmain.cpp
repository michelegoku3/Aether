// Supported xinput1_4.dll proxy: forwards Steam's public and private XInput
// calls to the *system* DLL and schedules AetherCore outside the loader lock.
// Unlike DLL forwarder strings, the explicit System32 path cannot resolve
// back to this proxy by accident. This module also forwards for non-Steam
// hosts, but the shared bootstrap will never inject into them.
#include <windows.h>
#include <cwchar>

// XInput is the only Steam bootstrap. Load the core from this DLL's directory
// on a worker after DllMain returns; never load it from an untrusted PATH.
namespace aether::proxy {

DWORD WINAPI LoadCoreOutsideLoaderLock(LPVOID parameter) {
    const auto self = static_cast<HMODULE>(parameter);
    wchar_t exePath[MAX_PATH] = {};
    wchar_t proxyPath[MAX_PATH] = {};
    const DWORD exeLen = GetModuleFileNameW(nullptr, exePath, MAX_PATH);
    const DWORD proxyLen = GetModuleFileNameW(self, proxyPath, MAX_PATH);
    if (!exeLen || exeLen >= MAX_PATH || !proxyLen || proxyLen >= MAX_PATH) {
        OutputDebugStringW(L"[Aether] XInput: module path unavailable/truncated; injection skipped.\n");
        return 0;
    }

    const wchar_t* exeName = std::wcsrchr(exePath, L'\\');
    wchar_t* proxyName = std::wcsrchr(proxyPath, L'\\');
    if (!exeName || !proxyName || _wcsicmp(exeName + 1, L"steam.exe") != 0) {
        return 0;  // Forward normally, but never inject into a game or helper.
    }

    const size_t exeDirLen = static_cast<size_t>(exeName - exePath);
    const size_t proxyDirLen = static_cast<size_t>(proxyName - proxyPath);
    if (exeDirLen != proxyDirLen || _wcsnicmp(exePath, proxyPath, exeDirLen) != 0) {
        OutputDebugStringW(L"[Aether] XInput is not beside steam.exe; injection skipped.\n");
        return 0;
    }
    constexpr wchar_t kCore[] = L"AetherCore.dll";
    if (proxyDirLen + 1 + (sizeof(kCore) / sizeof(wchar_t)) > MAX_PATH) {
        OutputDebugStringW(L"[Aether] XInput: AetherCore path too long.\n");
        return 0;
    }
    std::wcscpy(proxyName + 1, kCore);

    if (!LoadLibraryW(proxyPath)) {
        OutputDebugStringW(L"[Aether] XInput could not load AetherCore.dll from Steam root.\n");
    }
    return 0;
}

void Schedule(HMODULE self) {
    // No waits, I/O or LoadLibrary inside DllMain. The loader owns this proxy.
    HANDLE thread = CreateThread(nullptr, 0, LoadCoreOutsideLoaderLock, self, 0, nullptr);
    if (thread) CloseHandle(thread);
    else OutputDebugStringW(L"[Aether] XInput: could not start bootstrap thread.\n");
}

}  // namespace aether::proxy

namespace {

INIT_ONCE s_realOnce = INIT_ONCE_STATIC_INIT;
HMODULE s_self = nullptr;
HMODULE s_real = nullptr;

BOOL CALLBACK LoadRealXInput(PINIT_ONCE, PVOID, PVOID*) {
    wchar_t path[MAX_PATH] = {};
    const UINT length = GetSystemDirectoryW(path, MAX_PATH);
    constexpr wchar_t kName[] = L"\\xinput1_4.dll";
    if (!length || length >= MAX_PATH || length + sizeof(kName) / sizeof(wchar_t) > MAX_PATH) {
        return FALSE;  // InitOnceExecuteOnce can retry on a later call.
    }
    std::wcscpy(path + length, kName);
    HMODULE real = LoadLibraryExW(path, nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
    if (!real || real == s_self) {
        // Never bounce back into our own exports if the system DLL is missing.
        return FALSE;
    }
    s_real = real;
    return TRUE;
}

template <typename Fn>
Fn Resolve(const char* name) {
    if (!InitOnceExecuteOnce(&s_realOnce, LoadRealXInput, nullptr, nullptr)) return nullptr;
    return reinterpret_cast<Fn>(GetProcAddress(s_real, name));
}

template <typename Fn>
Fn Resolve(WORD ordinal) {
    if (!InitOnceExecuteOnce(&s_realOnce, LoadRealXInput, nullptr, nullptr)) return nullptr;
    return reinterpret_cast<Fn>(GetProcAddress(s_real, MAKEINTRESOURCEA(ordinal)));
}

}  // namespace

extern "C" {
DWORD WINAPI XInputGetState(DWORD index, void* state) {
    using Fn = DWORD(WINAPI*)(DWORD, void*);
    if (auto real = Resolve<Fn>("XInputGetState")) return real(index, state);
    return ERROR_DEVICE_NOT_CONNECTED;
}
DWORD WINAPI XInputSetState(DWORD index, void* vibration) {
    using Fn = DWORD(WINAPI*)(DWORD, void*);
    if (auto real = Resolve<Fn>("XInputSetState")) return real(index, vibration);
    return ERROR_DEVICE_NOT_CONNECTED;
}
DWORD WINAPI XInputGetCapabilities(DWORD index, DWORD flags, void* capabilities) {
    using Fn = DWORD(WINAPI*)(DWORD, DWORD, void*);
    if (auto real = Resolve<Fn>("XInputGetCapabilities")) return real(index, flags, capabilities);
    return ERROR_DEVICE_NOT_CONNECTED;
}
void WINAPI XInputEnable(BOOL enabled) {
    using Fn = void(WINAPI*)(BOOL);
    if (auto real = Resolve<Fn>("XInputEnable")) real(enabled);
}
DWORD WINAPI XInputGetAudioDeviceIds(DWORD index, LPWSTR renderId, UINT* renderCount,
                                    LPWSTR captureId, UINT* captureCount) {
    using Fn = DWORD(WINAPI*)(DWORD, LPWSTR, UINT*, LPWSTR, UINT*);
    if (auto real = Resolve<Fn>("XInputGetAudioDeviceIds"))
        return real(index, renderId, renderCount, captureId, captureCount);
    return ERROR_DEVICE_NOT_CONNECTED;
}
DWORD WINAPI XInputGetBatteryInformation(DWORD index, BYTE type, void* battery) {
    using Fn = DWORD(WINAPI*)(DWORD, BYTE, void*);
    if (auto real = Resolve<Fn>("XInputGetBatteryInformation")) return real(index, type, battery);
    return ERROR_DEVICE_NOT_CONNECTED;
}
DWORD WINAPI XInputGetKeystroke(DWORD index, DWORD reserved, void* keystroke) {
    using Fn = DWORD(WINAPI*)(DWORD, DWORD, void*);
    if (auto real = Resolve<Fn>("XInputGetKeystroke")) return real(index, reserved, keystroke);
    return ERROR_DEVICE_NOT_CONNECTED;
}

// Steam also imports private XInput ordinals (Big Picture/guide button).
// Keep each ordinal NONAME in the .def so an ordinal import remains ordinal.
DWORD WINAPI XInputOrdinal100(DWORD index, void* state) {
    using Fn = DWORD(WINAPI*)(DWORD, void*);
    if (auto real = Resolve<Fn>(100)) return real(index, state);
    return ERROR_DEVICE_NOT_CONNECTED;
}
DWORD WINAPI XInputOrdinal101(DWORD index, DWORD flags, void* result) {
    using Fn = DWORD(WINAPI*)(DWORD, DWORD, void*);
    if (auto real = Resolve<Fn>(101)) return real(index, flags, result);
    return ERROR_DEVICE_NOT_CONNECTED;
}
DWORD WINAPI XInputOrdinal102(DWORD index) {
    using Fn = DWORD(WINAPI*)(DWORD);
    if (auto real = Resolve<Fn>(102)) return real(index);
    return ERROR_DEVICE_NOT_CONNECTED;
}
DWORD WINAPI XInputOrdinal103(DWORD index) {
    using Fn = DWORD(WINAPI*)(DWORD);
    if (auto real = Resolve<Fn>(103)) return real(index);
    return ERROR_DEVICE_NOT_CONNECTED;
}
DWORD WINAPI XInputOrdinal104(DWORD index, void* info) {
    using Fn = DWORD(WINAPI*)(DWORD, void*);
    if (auto real = Resolve<Fn>(104)) return real(index, info);
    return ERROR_DEVICE_NOT_CONNECTED;
}
DWORD WINAPI XInputOrdinal108(DWORD index, void* a2, void* a3, void* a4, void* a5) {
    using Fn = DWORD(WINAPI*)(DWORD, void*, void*, void*, void*);
    if (auto real = Resolve<Fn>(108)) return real(index, a2, a3, a4, a5);
    return ERROR_DEVICE_NOT_CONNECTED;
}
}  // extern "C"

BOOL APIENTRY DllMain(HMODULE instance, DWORD reason, LPVOID /*reserved*/) {
    if (reason == DLL_PROCESS_ATTACH) {
        s_self = instance;
        DisableThreadLibraryCalls(instance);
        aether::proxy::Schedule(instance);
    }
    return TRUE;  // A missing AetherCore or XInput does not break Steam's load.
}
