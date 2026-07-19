#include "pch.h"
#include "utils/EnvReader.h"

#include <windows.h>

#include <algorithm>
#include <cstdint>
#include <cwchar>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "core/Constants.h"
#include "core/Logger.h"

namespace ac::env {
namespace {

constexpr const char* kModule = "EnvReader";

// ---------------------------------------------------------------------------
// NT API plumbing — just enough to walk the remote PEB without pulling in
// <winternl.h> or the full NTDDK.
// ---------------------------------------------------------------------------

struct ProcessBasicInformation {
    LONG     exitStatus;
    PVOID    pebBaseAddress;
    ULONG_PTR affinityMask;
    LONG     basePriority;
    ULONG_PTR uniqueProcessId;
    ULONG_PTR inheritedFromUniqueProcessId;
};

using NtQueryInformationProcess_t = LONG(NTAPI*)(HANDLE, ULONG, PVOID, ULONG, PULONG);

constexpr ULONG kProcessBasicInformation = 0;
constexpr ULONG kProcessWow64Information = 26;
constexpr std::size_t kMaxEnvironmentBytes = 64u * 1024u;

// ---------------------------------------------------------------------------
// Remote memory helpers
// ---------------------------------------------------------------------------

template <typename T>
std::optional<T> ReadRemoteValue(HANDLE process, const void* address) {
    T value{};
    SIZE_T bytesRead = 0;
    if (!ReadProcessMemory(process, address, &value, sizeof(value), &bytesRead) ||
        bytesRead != sizeof(value)) {
        return std::nullopt;
    }
    return value;
}

// ---------------------------------------------------------------------------
// PEB traversal: resolve the environment-block address in the remote process,
// first trying WOW64 (32-bit PEB) then native (64-bit PEB).
// ---------------------------------------------------------------------------

std::optional<void*> ReadNativeEnvironmentAddress(HANDLE process) {
    HMODULE ntdll = GetModuleHandleW(L"ntdll.dll");
    if (!ntdll) return std::nullopt;

    auto query = reinterpret_cast<NtQueryInformationProcess_t>(
        GetProcAddress(ntdll, "NtQueryInformationProcess"));
    if (!query) return std::nullopt;

    ProcessBasicInformation info{};
    if (query(process, kProcessBasicInformation, &info, sizeof(info), nullptr) < 0 ||
        !info.pebBaseAddress) {
        return std::nullopt;
    }

    // x64: PEB → +0x20 → ProcessParameters → +0x80 → Environment
    auto params = ReadRemoteValue<void*>(
        process, reinterpret_cast<const std::uint8_t*>(info.pebBaseAddress) + 0x20);
    if (!params || !*params) return std::nullopt;

    auto env = ReadRemoteValue<void*>(
        process, reinterpret_cast<const std::uint8_t*>(*params) + 0x80);
    if (!env || !*env) return std::nullopt;

    return *env;
}

std::optional<void*> ReadWow64EnvironmentAddress(HANDLE process) {
    HMODULE ntdll = GetModuleHandleW(L"ntdll.dll");
    if (!ntdll) return std::nullopt;

    auto query = reinterpret_cast<NtQueryInformationProcess_t>(
        GetProcAddress(ntdll, "NtQueryInformationProcess"));
    if (!query) return std::nullopt;

    ULONG_PTR peb32 = 0;
    if (query(process, kProcessWow64Information, &peb32, sizeof(peb32), nullptr) < 0 ||
        peb32 == 0) {
        return std::nullopt;
    }

    // WOW64: PEB32 → +0x10 → ProcessParameters → +0x48 → Environment
    auto params = ReadRemoteValue<std::uint32_t>(
        process, reinterpret_cast<const std::uint8_t*>(static_cast<std::uintptr_t>(peb32)) + 0x10);
    if (!params || *params == 0) return std::nullopt;

    auto env = ReadRemoteValue<std::uint32_t>(
        process, reinterpret_cast<const std::uint8_t*>(static_cast<std::uintptr_t>(*params)) + 0x48);
    if (!env || *env == 0) return std::nullopt;

    return reinterpret_cast<void*>(static_cast<std::uintptr_t>(*env));
}

std::optional<std::string> FindEnvironmentValue(const std::vector<wchar_t>& block,
                                                std::wstring_view name) {
    std::size_t offset = 0;
    while (offset < block.size() && block[offset] != L'\0') {
        const auto begin = block.begin() + static_cast<std::ptrdiff_t>(offset);
        const auto end = std::find(begin, block.end(), L'\0');
        if (end == block.end()) return std::nullopt;

        const wchar_t* entry = &*begin;
        const std::size_t length = static_cast<std::size_t>(end - begin);
        if (length > name.size() &&
            entry[name.size()] == L'=' &&
            _wcsnicmp(entry, name.data(), name.size()) == 0) {
            std::string value;
            value.reserve(length - name.size() - 1);
            for (const wchar_t* cur = entry + name.size() + 1; *cur; ++cur) {
                if (*cur > 0x7F) return std::nullopt;
                value.push_back(static_cast<char>(*cur));
            }
            return value;
        }
        offset += length + 1;
    }
    return std::nullopt;
}

std::optional<std::uint64_t> ParseUnsigned(std::string_view text) {
    if (text.empty()) return std::nullopt;
    std::uint64_t value = 0;
    for (unsigned char ch : text) {
        if (ch < '0' || ch > '9') return std::nullopt;
        const std::uint64_t digit = static_cast<std::uint64_t>(ch - '0');
        if (value > (UINT64_MAX - digit) / 10) return std::nullopt;
        value = value * 10 + digit;
    }
    return value;
}

steam::AppId AppIdFromGameId(std::uint64_t gameId) {
    return static_cast<steam::AppId>(gameId & constants::kGameIdAppIdMask);
}

std::optional<steam::AppId> AppIdFromEnvValue(const std::vector<wchar_t>& env,
                                              std::wstring_view name,
                                              bool encodedGameId) {
    auto text = FindEnvironmentValue(env, name);
    if (!text) return std::nullopt;

    auto parsed = ParseUnsigned(*text);
    if (!parsed) return std::nullopt;

    steam::AppId appId = encodedGameId ? AppIdFromGameId(*parsed)
                                       : static_cast<steam::AppId>(*parsed);
    return appId != 0 ? std::optional<steam::AppId>(appId) : std::nullopt;
}

}  // namespace

std::optional<std::vector<wchar_t>> ReadEnvironmentBlock(HANDLE process) {
    std::optional<void*> env = ReadWow64EnvironmentAddress(process);
    if (!env) env = ReadNativeEnvironmentAddress(process);
    if (!env) {
        AC_LOG_DEBUG_ONCE(kModule, "Unable to resolve PEB environment address for process handle.");
        return std::nullopt;
    }

    MEMORY_BASIC_INFORMATION mbi{};
    if (!VirtualQueryEx(process, *env, &mbi, sizeof(mbi)) || mbi.RegionSize == 0) {
        AC_LOG_DEBUG_ONCE(kModule, "VirtualQueryEx failed on PEB environment block.");
        return std::nullopt;
    }

    const auto base = reinterpret_cast<std::uintptr_t>(mbi.BaseAddress);
    const auto ptr  = reinterpret_cast<std::uintptr_t>(*env);
    if (ptr < base) return std::nullopt;

    const std::size_t offset = static_cast<std::size_t>(ptr - base);
    if (offset >= mbi.RegionSize) return std::nullopt;

    const std::size_t maxBytes = (std::min)(mbi.RegionSize - offset, kMaxEnvironmentBytes);
    if (maxBytes < sizeof(wchar_t) * 2) return std::nullopt;

    std::vector<wchar_t> data(maxBytes / sizeof(wchar_t));
    SIZE_T bytesRead = 0;
    if (!ReadProcessMemory(process, *env, data.data(),
                           data.size() * sizeof(wchar_t), &bytesRead) ||
        bytesRead < sizeof(wchar_t) * 2) {
        AC_LOG_DEBUG_ONCE(kModule, "ReadProcessMemory failed for PEB environment block.");
        return std::nullopt;
    }

    data.resize(bytesRead / sizeof(wchar_t));

    for (std::size_t i = 1; i < data.size(); ++i) {
        if (data[i - 1] == L'\0' && data[i] == L'\0') {
            data.resize(i + 1);
            return data;
        }
    }
    return std::nullopt;
}

std::optional<EnvAppIds> ReadSteamEnvAppIds(HANDLE process) {
    auto env = ReadEnvironmentBlock(process);
    if (!env) return std::nullopt;

    EnvAppIds ids{};

    if (auto appId = AppIdFromEnvValue(*env, L"SteamOverlayGameId", true)) {
        ids.steamOverlayGameId = *appId;
        ids.selected = *appId;
        ids.source = "SteamOverlayGameId";
    }
    if (auto appId = AppIdFromEnvValue(*env, L"SteamGameId", true)) {
        ids.steamGameId = *appId;
        if (!ids.selected) {
            ids.selected = *appId;
            ids.source = "SteamGameId";
        }
    }
    if (auto appId = AppIdFromEnvValue(*env, L"SteamAppId", false)) {
        ids.steamAppId = *appId;
        if (!ids.selected) {
            ids.selected = *appId;
            ids.source = "SteamAppId";
        }
    }

    if (!ids.selected) {
        AC_LOG_DEBUG_ONCE(kModule, "Environment block read successfully, but no Steam AppId variables found.");
        return std::nullopt;
    }
    AC_LOG_DEBUG_ONCE(kModule, "Resolved AppId %u from environment variable %s.", ids.selected, ids.source ? ids.source : "-");
    return ids;
}

}  // namespace ac::env
