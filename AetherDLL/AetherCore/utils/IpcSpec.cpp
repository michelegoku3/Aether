#include "pch.h"
#include "utils/IpcSpec.h"

#include <toml++/toml.hpp>

#include <cctype>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <limits>
#include <sstream>
#include <string>
#include <unordered_map>
#include <utility>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "utils/PatternDownloader.h"

namespace ac::ipcspec {
namespace {

constexpr const char* kModule = "IpcSpec";
constexpr const char* kSubdir = "steamclientipc";

std::string CachePath() {
    return g_state.patternDir + "\\steamclientipc\\" + g_state.steamclientSha + ".toml";
}

// Builds "IClientUser::GetSteamID" from the TOML table path.
std::string QualifiedName(std::string_view iface, std::string_view method) {
    std::string out;
    out.reserve(iface.size() + 2 + method.size());
    out.append(iface);
    out.append("::");
    out.append(method);
    return out;
}

bool ParseHex32(std::string_view text, std::uint32_t& out) {
    if (text.empty()) return false;
    std::string value(text);
    if (value.size() >= 2 && value[0] == '0' &&
        (value[1] == 'x' || value[1] == 'X')) {
        value.erase(0, 2);
    }
    if (value.empty() || value.size() > 8) return false;
    for (unsigned char c : value) {
        if (!std::isxdigit(c)) return false;
    }

    char* end = nullptr;
    const unsigned long parsed = std::strtoul(value.c_str(), &end, 16);
    if (!end || *end != '\0' || parsed == 0 ||
        parsed > std::numeric_limits<std::uint32_t>::max()) {
        return false;
    }
    out = static_cast<std::uint32_t>(parsed);
    return true;
}

// Parses the IPC spec into temporary maps. State is published only after the
// full document has produced at least one valid method entry.
bool ParseToml(const std::string& body) {
    std::unordered_map<std::string, std::uint8_t> interfaceIds;
    std::unordered_map<std::string, std::uint32_t> hashes;

    try {
        auto tbl = toml::parse(body);
        for (const auto& [ifaceKey, ifaceNode] : tbl) {
            const std::string ifaceName(ifaceKey.str());
            auto* ifaceTbl = ifaceNode.as_table();
            if (!ifaceTbl || ifaceName.empty()) continue;

            if (auto id = (*ifaceTbl)["interface_id"].value<std::int64_t>()) {
                if (*id <= 0 || *id > 255) {
                    AC_LOG_WARN(kModule, "Invalid interface_id for %s.", ifaceName.c_str());
                    continue;
                }
                interfaceIds.emplace(ifaceName, static_cast<std::uint8_t>(*id));
            }

            for (const auto& [methodKey, methodNode] : *ifaceTbl) {
                const std::string methodName(methodKey.str());
                auto* methodTbl = methodNode.as_table();
                if (!methodTbl || methodName.empty()) continue;

                auto hashStr = (*methodTbl)["funcHash"].value<std::string>();
                if (!hashStr) continue;

                std::uint32_t hash = 0;
                if (!ParseHex32(*hashStr, hash)) {
                    AC_LOG_WARN(kModule, "Invalid funcHash for %s::%s.",
                                ifaceName.c_str(), methodName.c_str());
                    continue;
                }
                hashes.emplace(QualifiedName(ifaceName, methodName), hash);
            }
        }
    } catch (const toml::parse_error& e) {
        AC_LOG_WARN(kModule, "TOML parse error: %s", e.what());
        return false;
    }

    if (hashes.empty()) return false;
    g_state.ipcSpec.interfaceIds = std::move(interfaceIds);
    g_state.ipcSpec.hashes = std::move(hashes);
    return true;
}

// Reads the spec from the local cache directory.
bool LoadFromCache() {
    const std::string path = CachePath();
    std::error_code ec;
    if (!std::filesystem::exists(path, ec)) return false;

    std::ifstream f(path, std::ios::binary);
    if (!f) return false;
    std::ostringstream ss;
    ss << f.rdbuf();
    if (ss.str().empty()) return false;
    return ParseToml(ss.str());
}

}  // namespace

bool Init() {
    if (g_state.ipcSpec.loaded) return true;  // already done

    if (g_state.steamclientSha.empty() || g_state.steamclientSha.size() != 64) {
        AC_LOG_WARN(kModule, "No steamclient SHA; skipping IPC spec load.");
        return false;
    }

    // 1. Try local cache first.
    if (LoadFromCache()) {
        g_state.ipcSpec.loaded = true;
        AC_LOG_INFO(kModule, "Loaded IPC spec from cache (%zu entries).",
                    g_state.ipcSpec.hashes.size());
        return true;
    }

    // 2. Download from the same mirror chain as pattern TOMLs.
    const std::string outPath = CachePath();
    {
        std::error_code ec;
        std::filesystem::create_directories(
            std::filesystem::path(outPath).parent_path(), ec);
    }

    std::string source;
    if (!downloader::Download(kSubdir, g_state.steamclientSha, outPath, &source)) {
        AC_LOG_WARN(kModule, "IPC spec download failed for SHA %s; "
                    "falling back to compile-time hashes.",
                    g_state.steamclientSha.c_str());
        return false;
    }

    // 3. Parse the freshly downloaded file.
    if (!LoadFromCache()) {
        AC_LOG_WARN(kModule, "IPC spec parse failed after download.");
        return false;
    }

    g_state.ipcSpec.loaded = true;
    AC_LOG_INFO(kModule, "Loaded IPC spec from %s (%zu entries).",
                source.c_str(), g_state.ipcSpec.hashes.size());
    return true;
}

std::optional<std::uint8_t> ResolveInterfaceId(const char* interfaceName) {
    if (!g_state.ipcSpec.loaded || !interfaceName) return std::nullopt;
    auto it = g_state.ipcSpec.interfaceIds.find(interfaceName);
    if (it != g_state.ipcSpec.interfaceIds.end()) return it->second;
    return std::nullopt;
}

std::optional<std::uint32_t> ResolveHash(const char* qualifiedName) {
    if (!g_state.ipcSpec.loaded || !qualifiedName) return std::nullopt;
    auto it = g_state.ipcSpec.hashes.find(qualifiedName);
    if (it != g_state.ipcSpec.hashes.end()) return it->second;
    return std::nullopt;
}

bool IsLoaded() {
    return g_state.ipcSpec.loaded;
}

}  // namespace ac::ipcspec
