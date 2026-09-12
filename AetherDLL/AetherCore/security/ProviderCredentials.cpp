#include "pch.h"
#include "security/ProviderCredentials.h"

#include <windows.h>
#include <wincrypt.h>

#include <cctype>
#include <cstdint>
#include <fstream>
#include <iterator>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "hooks/wire/BackupIo.h"

#pragma comment(lib, "crypt32.lib")

namespace ac::security {
namespace {

constexpr const char* kModule = "ProviderCredentials";

std::string Trim(std::string value) {
    while (!value.empty() && std::isspace(static_cast<unsigned char>(value.front()))) {
        value.erase(value.begin());
    }
    while (!value.empty() && std::isspace(static_cast<unsigned char>(value.back()))) {
        value.pop_back();
    }
    return value;
}

// ProviderCredentials is intentionally parsed without a general JSON
// dependency: the Rust writer emits a small object with string fields, and the
// DLL only needs one field. Escaped JSON strings are decoded for the limited
// escapes that can occur in a credential (including unicode is not needed for
// an API key and is rejected rather than misinterpreted).
std::optional<std::string> JsonStringField(std::string_view json,
                                            std::string_view field) {
    const std::string needle = "\"" + std::string(field) + "\"";
    const std::size_t key = json.find(needle);
    if (key == std::string_view::npos) return std::nullopt;

    std::size_t cursor = key + needle.size();
    while (cursor < json.size() && std::isspace(static_cast<unsigned char>(json[cursor]))) ++cursor;
    if (cursor >= json.size() || json[cursor] != ':') return std::nullopt;
    ++cursor;
    while (cursor < json.size() && std::isspace(static_cast<unsigned char>(json[cursor]))) ++cursor;
    if (cursor >= json.size() || json[cursor] != '"') return std::nullopt;
    ++cursor;

    std::string value;
    value.reserve(64);
    bool escaped = false;
    for (; cursor < json.size(); ++cursor) {
        const char c = json[cursor];
        if (escaped) {
            escaped = false;
            switch (c) {
            case '"': value.push_back('"'); break;
            case '\\': value.push_back('\\'); break;
            case '/': value.push_back('/'); break;
            case 'b': value.push_back('\b'); break;
            case 'f': value.push_back('\f'); break;
            case 'n': value.push_back('\n'); break;
            case 'r': value.push_back('\r'); break;
            case 't': value.push_back('\t'); break;
            default: return std::nullopt;
            }
            continue;
        }
        if (c == '\\') {
            escaped = true;
            continue;
        }
        if (c == '"') return value;
        value.push_back(c);
    }
    return std::nullopt;
}

std::optional<std::string> Unprotect(std::vector<std::uint8_t> encrypted) {
    if (encrypted.empty()) return std::nullopt;

    DATA_BLOB input{};
    input.pbData = encrypted.data();
    input.cbData = static_cast<DWORD>(encrypted.size());
    DATA_BLOB output{};

    if (!CryptUnprotectData(&input, nullptr, nullptr, nullptr, nullptr,
                            CRYPTPROTECT_UI_FORBIDDEN, &output)) {
        AC_LOG_WARN(kModule,
                    "Could not decrypt provider credentials with Windows DPAPI (error=%lu).",
                    static_cast<unsigned long>(GetLastError()));
        return std::nullopt;
    }

    std::string plain(reinterpret_cast<const char*>(output.pbData), output.cbData);
    if (output.pbData) LocalFree(output.pbData);
    return plain;
}

}  // namespace

std::optional<std::string> ReadHubcapApiKey() {
    const std::string deskData = backup::io::CachedDeskDataDir();
    if (deskData.empty()) {
        AC_LOG_DEBUG(kModule, "Provider credentials unavailable: AetherData path is not configured.");
        return std::nullopt;
    }

    const std::string path = deskData + "\\config\\provider_credentials.dat";
    std::ifstream input(path, std::ios::binary);
    if (!input.is_open()) {
        AC_LOG_DEBUG(kModule, "Provider credentials file is not present.");
        return std::nullopt;
    }

    std::vector<std::uint8_t> encrypted(
        (std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
    auto plain = Unprotect(std::move(encrypted));
    if (!plain) return std::nullopt;

    auto key = JsonStringField(*plain, "hubcap_api_key");
    SecureZeroMemory(plain->data(), plain->size());
    if (!key) return std::nullopt;

    *key = Trim(std::move(*key));
    if (key->empty() || key->find('\r') != std::string::npos ||
        key->find('\n') != std::string::npos) {
        AC_LOG_WARN(kModule, "Hubcap credential rejected because it is empty or contains invalid header characters.");
        return std::nullopt;
    }
    return key;
}

}  // namespace ac::security
