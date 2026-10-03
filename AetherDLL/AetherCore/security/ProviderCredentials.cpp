#include "pch.h"
#include "utils/Strings.h"
#include "security/ProviderCredentials.h"

#include <windows.h>
#include <wincrypt.h>

#include <cctype>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "utils/JsonStringField.h"
#include "hooks/wire/BackupIo.h"

#pragma comment(lib, "crypt32.lib")

namespace ac::security {
namespace {

constexpr const char* kModule = "ProviderCredentials";

using strings::Trim;


// ProviderCredentials is intentionally parsed without a general JSON
// dependency: the Rust writer emits a small object with string fields, and the
// DLL only needs one field. Escape handling lives in the shared escape-aware
// puller (utils/JsonStringField.h, jsonutil::PullEscapedStringField): strict,
// rejects \u and unknown escapes rather than misinterpreting them.
std::optional<std::string> JsonStringField(std::string_view json,
                                            std::string_view field) {
    std::string value;
    if (!jsonutil::PullEscapedStringField(json, field, value)) return std::nullopt;
    return value;   // empty string stays empty: caller rejects after Trim
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

    const std::filesystem::path path =
        std::filesystem::path(deskData) / "config" / "provider_credentials.dat";
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
