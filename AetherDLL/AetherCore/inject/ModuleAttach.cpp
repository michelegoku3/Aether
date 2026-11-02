#include "pch.h"
#include "inject/ModuleAttach.h"

#include "core/Constants.h"
#include "core/Logger.h"
#include "utils/Strings.h"

namespace ac {
namespace {

bool IsExpectedModule(HMODULE module, const std::string& expectedPath, const char* logModule) {
    std::wstring path(32768, L'\0');
    const DWORD length = GetModuleFileNameW(module, path.data(), static_cast<DWORD>(path.size()));
    if (length == 0 || length >= path.size())
        return false;
    if (_wcsicmp(path.c_str(), strings::Widen(expectedPath).c_str()) == 0)
        return true;
    AC_LOG_WARN_ONCE(logModule, "A module was mapped from another directory; waiting for Steam's copy at %s.",
                     expectedPath.c_str());
    return false;
}

} // namespace

HMODULE WaitForMappedModule(const char* baseName, const std::string& expectedPath, std::uint32_t timeoutMs,
                            const char* logModule, std::string& outcome) {
    if (expectedPath.empty() || strings::Widen(expectedPath).empty()) {
        outcome = "path-invalid";
        AC_LOG_ERROR(logModule, "Module path is not resolved; aborting attach.");
        return nullptr;
    }

    const ULONGLONG start = GetTickCount64();
    for (;;) {
        HMODULE module = GetModuleHandleA(baseName);
        if (module && IsExpectedModule(module, expectedPath, logModule)) {
            outcome = "mapped";
            AC_LOG_INFO(logModule, "%s mapped by Steam after %llu ms (handle 0x%p).", baseName,
                        GetTickCount64() - start, module);
            return module;
        }
        if (GetTickCount64() - start >= timeoutMs)
            break;
        Sleep(static_cast<DWORD>(constants::kSteamclientAttachPollMs));
    }

    outcome = "not-mapped";
    // WARN, not ERROR: a slow mapping is a normal boot, and every caller keeps
    // probing with a zero budget from a worker or the late retry.
    AC_LOG_WARN(logModule, "Steam has not mapped %s after %u ms; the batch is deferred.", expectedPath.c_str(),
                timeoutMs);
    return nullptr;
}

} // namespace ac
