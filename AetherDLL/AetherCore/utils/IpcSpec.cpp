#include "pch.h"
#include "utils/IpcSpec.h"

#include <filesystem>
#include <fstream>
#include <sstream>
#include <string>
#include <unordered_map>
#include <utility>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "utils/IpcSpecParse.h"
#include "utils/PatternDownloader.h"

namespace ac::ipcspec {
namespace {

constexpr const char* kModule = "IpcSpec";
constexpr const char* kSubdir = "steamclientipc";

std::string CachePath() {
    return g_state.patternDir + "\\steamclientipc\\" + g_state.steamclientSha + ".toml";
}

// Parses the IPC spec via the pure parser (utils/IpcSpecParse) and publishes
// the result only after the full document produced at least one valid method.
bool ParseToml(const std::string& body) {
    std::unordered_map<std::string, std::uint8_t> interfaceIds;
    std::unordered_map<std::string, MethodSpec> methods;
    if (!ParseSpecToml(body, interfaceIds, methods)) return false;
    g_state.ipcSpec.interfaceIds = std::move(interfaceIds);
    g_state.ipcSpec.methods = std::move(methods);
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
    static std::mutex initMutex;  // private initialization lifecycle
    std::lock_guard lock(initMutex);
    if (g_state.ipcSpec.loaded) return true;  // already done

    if (g_state.steamclientSha.empty() || g_state.steamclientSha.size() != 64) {
        AC_LOG_WARN(kModule, "No steamclient SHA; skipping IPC spec load.");
        return false;
    }

    // 1. Try local cache first.
    if (LoadFromCache()) {
        g_state.ipcSpec.loaded = true;
        AC_LOG_INFO(kModule, "Loaded IPC spec from cache (%zu entries).",
                    g_state.ipcSpec.methods.size());
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
                source.c_str(), g_state.ipcSpec.methods.size());
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
    auto it = g_state.ipcSpec.methods.find(qualifiedName);
    if (it != g_state.ipcSpec.methods.end()) return it->second.hash;
    return std::nullopt;
}

std::optional<MethodSpec> ResolveMethodSpec(const char* qualifiedName) {
    if (!g_state.ipcSpec.loaded || !qualifiedName) return std::nullopt;
    auto it = g_state.ipcSpec.methods.find(qualifiedName);
    if (it != g_state.ipcSpec.methods.end()) return it->second;
    return std::nullopt;
}

bool IsLoaded() {
    return g_state.ipcSpec.loaded;
}

}  // namespace ac::ipcspec
