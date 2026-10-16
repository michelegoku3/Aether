#include "pch.h"
#include "credentials/CredentialStore.h"

#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "utils/Strings.h"

namespace ac::credential {
namespace {

constexpr const char* kModule = "Credential";
constexpr DWORD kInitialReadBytes = 2048;

std::string AppKeyPath(steam::AppId appId) { return "Software\\Valve\\Steam\\Apps\\" + std::to_string(appId); }

// Registry key/value names in this TU are ASCII literals (Valve paths,
// "SteamID", "AppTicket"); values may be non-ASCII (SteamPath). UTF-8
// inside, *W at the boundary: Widen is exact on the ASCII names and strict
// on the values.
std::wstring WidenKey(const char* narrow) { return ac::strings::Widen(narrow ? narrow : ""); }

std::string ReadRegString(HKEY root, const char* subKey, const char* value) {
    // Two-phase wide read: no MAX_PATH truncation, no codepage mangling.
    const std::wstring wSub = WidenKey(subKey);
    const std::wstring wValue = WidenKey(value);
    DWORD sizeBytes = 0;
    LSTATUS status = RegGetValueW(root, wSub.c_str(), wValue.c_str(), RRF_RT_REG_SZ, nullptr, nullptr, &sizeBytes);
    if (status != ERROR_SUCCESS || sizeBytes == 0 || sizeBytes > 32768 * sizeof(wchar_t)) {
        AC_LOG_DEBUG_ONCE(kModule, "RegGetValueW string missing or unreadable: %s\\%s (status=%ld)",
                          subKey ? subKey : "-", value ? value : "-", status);
        return {};
    }
    std::wstring wide(sizeBytes / sizeof(wchar_t), L'\0');
    status = RegGetValueW(root, wSub.c_str(), wValue.c_str(), RRF_RT_REG_SZ, nullptr, wide.data(), &sizeBytes);
    if (status != ERROR_SUCCESS) {
        AC_LOG_DEBUG_ONCE(kModule, "RegGetValueW string missing or unreadable: %s\\%s (status=%ld)",
                          subKey ? subKey : "-", value ? value : "-", status);
        return {};
    }
    wide.resize((std::min)(wide.size(), sizeBytes / sizeof(wchar_t)));
    if (!wide.empty() && wide.back() == L'\0')
        wide.pop_back();
    return ac::strings::Narrow(wide);
}

bool WriteRegString(const char* subKey, const char* valueName, const std::string& data) {
    if (!subKey || !valueName)
        return false;
    HKEY key = nullptr;
    if (RegCreateKeyExW(HKEY_CURRENT_USER, WidenKey(subKey).c_str(), 0, nullptr, 0, KEY_WRITE, nullptr, &key,
                        nullptr) != ERROR_SUCCESS) {
        AC_LOG_DEBUG(kModule, "WriteRegString: key create failed for %s\\%s.", subKey, valueName);
        return false;
    }
    // For REG_SZ, the cbData argument must include the trailing NUL
    // (per Microsoft docs). Widened size counts visible chars, so add 1.
    const std::wstring wide = ac::strings::Widen(data);
    LSTATUS status =
        RegSetValueExW(key, WidenKey(valueName).c_str(), 0, REG_SZ, reinterpret_cast<const BYTE*>(wide.c_str()),
                       static_cast<DWORD>((wide.size() + 1) * sizeof(wchar_t)));
    RegCloseKey(key);
    if (status != ERROR_SUCCESS) {
        AC_LOG_DEBUG(kModule, "WriteRegString: set failed for %s\\%s (status=%ld).", subKey, valueName, status);
        return false;
    }
    return true;
}

bool WriteBinary(steam::AppId appId, const char* valueName, const std::vector<std::uint8_t>& data) {
    HKEY key = nullptr;
    const std::string path = AppKeyPath(appId);
    if (RegCreateKeyExW(HKEY_CURRENT_USER, WidenKey(path.c_str()).c_str(), 0, nullptr, 0, KEY_WRITE, nullptr, &key,
                        nullptr) != ERROR_SUCCESS) {
        AC_LOG_ERROR(kModule, "Could not open/create key %s.", path.c_str());
        return false;
    }
    LSTATUS status =
        RegSetValueExW(key, WidenKey(valueName).c_str(), 0, REG_BINARY, data.data(), static_cast<DWORD>(data.size()));
    RegCloseKey(key);
    if (status != ERROR_SUCCESS) {
        AC_LOG_ERROR(kModule, "Write %s for app %u failed (status=%ld).", valueName, appId, status);
        return false;
    }
    AC_LOG_INFO(kModule, "Wrote %s for app %u (%zu bytes).", valueName, appId, data.size());
    return true;
}

std::vector<std::uint8_t> ReadBinary(steam::AppId appId, const char* valueName) {
    HKEY key = nullptr;
    const std::string path = AppKeyPath(appId);
    if (RegOpenKeyExW(HKEY_CURRENT_USER, WidenKey(path.c_str()).c_str(), 0, KEY_READ, &key) != ERROR_SUCCESS) {
        AC_LOG_DEBUG_ONCE(kModule, "Registry key unreadable for app %u path %s.", appId, path.c_str());
        return {};
    }
    std::vector<std::uint8_t> buffer(kInitialReadBytes);
    DWORD size = static_cast<DWORD>(buffer.size());
    DWORD type = 0;
    const std::wstring wValue = WidenKey(valueName);
    LSTATUS status = RegQueryValueExW(key, wValue.c_str(), nullptr, &type, buffer.data(), &size);
    for (int retry = 0; retry < 3 && status == ERROR_MORE_DATA; ++retry) {
        buffer.resize(size);
        status = RegQueryValueExW(key, wValue.c_str(), nullptr, &type, buffer.data(), &size);
    }
    RegCloseKey(key);
    if (status != ERROR_SUCCESS || type != REG_BINARY) {
        AC_LOG_DEBUG_ONCE(kModule, "Registry binary %s missing/invalid for app %u (status=%ld type=%lu).", valueName,
                          appId, status, type);
        return {};
    }
    buffer.resize(size);
    AC_LOG_DEBUG_ONCE(kModule, "Read %s for app %u (%u bytes).", valueName, appId, size);
    return buffer;
}

std::uint64_t ParseDecimal(std::string_view s) {
    if (s.empty())
        return 0;
    // Manual accumulation: string_view data is not guaranteed NUL-terminated,
    // and strtoull would silently clamp on overflow. Fail closed instead.
    std::uint64_t value = 0;
    for (char c : s) {
        if (c < '0' || c > '9')
            return 0;
        const std::uint64_t digit = static_cast<std::uint64_t>(c - '0');
        if (value > (UINT64_MAX - digit) / 10)
            return 0;
        value = value * 10 + digit;
    }
    return value;
}

} // namespace

bool WriteAppOwnershipTicket(steam::AppId appId, const std::vector<std::uint8_t>& data) {
    return WriteBinary(appId, "AppTicket", data);
}

bool WriteEncryptedTicket(steam::AppId appId, const std::vector<std::uint8_t>& data) {
    return WriteBinary(appId, "ETicket", data);
}

std::vector<std::uint8_t> ReadAppOwnershipTicket(steam::AppId appId) {
    {
        std::lock_guard<std::mutex> lock(g_state.configStoreTicketMutex);
        auto it = g_state.configStoreAppTickets.find(appId);
        if (it != g_state.configStoreAppTickets.end()) {
            AC_LOG_DEBUG_ONCE(kModule, "Read cached ConfigStore AppTicket for app %u (%zu bytes).", appId,
                              it->second.size());
            return it->second;
        }
    }
    return ReadBinary(appId, "AppTicket");
}

std::vector<std::uint8_t> ReadEncryptedTicket(steam::AppId appId) { return ReadBinary(appId, "ETicket"); }

std::uint32_t ReadActiveUserId() {
    DWORD accountId = 0, size = sizeof(accountId);
    if (RegGetValueW(HKEY_CURRENT_USER, L"Software\\Valve\\Steam\\ActiveProcess", L"ActiveUser", RRF_RT_REG_DWORD,
                     nullptr, &accountId, &size) == ERROR_SUCCESS &&
        accountId != 0) {
        return accountId;
    }
    AC_LOG_DEBUG_ONCE(kModule, "ActiveUser value not present or 0 in ActiveProcess registry key.");
    return 0;
}

std::uint64_t ReadAppSteamIdValue(steam::AppId appId) {
    const std::string subKey = AppKeyPath(appId);
    return ParseDecimal(ReadRegString(HKEY_CURRENT_USER, subKey.c_str(), "SteamID"));
}

bool WriteAppSteamIdValue(steam::AppId appId, std::uint64_t steamId) {
    if (appId == 0 || steamId == 0) {
        AC_LOG_DEBUG(kModule, "WriteAppSteamIdValue: refused app=%u steamId=%llu.", appId,
                     static_cast<unsigned long long>(steamId));
        return false;
    }
    const std::string subKey = AppKeyPath(appId);
    // SteamID64 fits in 17 decimal digits; the buffer is comfortably large.
    char buf[32];
    const int len = std::snprintf(buf, sizeof(buf), "%llu", static_cast<unsigned long long>(steamId));
    if (len <= 0 || static_cast<std::size_t>(len) >= sizeof(buf)) {
        AC_LOG_DEBUG(kModule, "WriteAppSteamIdValue: format overflow for app=%u.", appId);
        return false;
    }
    if (!WriteRegString(subKey.c_str(), "SteamID", std::string(buf))) {
        return false;
    }
    AC_LOG_DEBUG(kModule, "WriteAppSteamIdValue: app=%u -> %llu.", appId, static_cast<unsigned long long>(steamId));
    return true;
}

std::string ReadSteamPath() { return ReadRegString(HKEY_CURRENT_USER, "Software\\Valve\\Steam", "SteamPath"); }

bool CacheConfigStoreAppOwnershipTicket(steam::AppId appId, const std::vector<std::uint8_t>& data) {
    if (data.empty()) {
        AC_LOG_WARN(kModule, "Attempted to cache empty ConfigStore ticket for app %u.", appId);
        return false;
    }
    std::lock_guard<std::mutex> lock(g_state.configStoreTicketMutex);
    g_state.configStoreAppTickets[appId] = data;
    AC_LOG_INFO(kModule, "Cached ConfigStore AppTicket for app %u (%zu bytes).", appId, data.size());
    return true;
}

std::size_t CachedConfigStoreTicketCount() {
    std::lock_guard<std::mutex> lock(g_state.configStoreTicketMutex);
    return g_state.configStoreAppTickets.size();
}

} // namespace ac::credential
