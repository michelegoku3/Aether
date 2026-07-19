#include "pch.h"
#include "utils/IpcSpec.h"

#include <toml++/toml.hpp>

#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <sstream>
#include <string>

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

// Parses the IPC spec TOML body into g_state.ipcSpec.hashes.
// Format (compatible with the KoriaPolis repo):
//   [IClientUser]
//   interface_id = 1
//   [IClientUser.GetSteamID]
//   funcHash = "0xD6FC3200"
//   fencepost = "0x..."
//   argc = 0
bool ParseToml(const std::string& body) {
    try {
        auto tbl = toml::parse(body);
        for (const auto& [ifaceKey, ifaceNode] : tbl) {
            auto* ifaceTbl = ifaceNode.as_table();
            if (!ifaceTbl) continue;

            for (const auto& [methodKey, methodNode] : *ifaceTbl) {
                auto* methodTbl = methodNode.as_table();
                if (!methodTbl) continue;

                // Only method sub-tables with a funcHash field are relevant.
                auto hashStr = methodTbl->get_as<toml::value<std::string>>("funcHash");
                if (!hashStr) continue;

                const char* start = hashStr->get().c_str();
                char* end = nullptr;
                const std::uint32_t hash = std::strtoul(start, &end, 16);
                if (end == start) continue;  // parse failure

                g_state.ipcSpec.hashes.emplace(
                    QualifiedName(ifaceKey.str(), methodKey.str()), hash);
            }
        }
    } catch (const toml::parse_error& e) {
        AC_LOG_WARN(kModule, "TOML parse error: %s", e.what());
        return false;
    }
    return !g_state.ipcSpec.hashes.empty();
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
