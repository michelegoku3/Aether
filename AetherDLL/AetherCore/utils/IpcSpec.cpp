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
#include "utils/PatternSidecar.h"
#include "utils/PatternState.h"
#include "utils/WinFile.h"

namespace ac::ipcspec {
namespace {

constexpr const char* kModule = "IpcSpec";
constexpr const char* kSubdir = "steamclientipc";
// A refresh runs beside the pattern tables at boot; keep the probe short so an
// unresponsive network cannot delay hook installation.
constexpr int kRefreshTimeoutSec = 5;

std::string CachePath() { return g_state.patternDir + "\\steamclientipc\\" + g_state.steamclientSha + ".toml"; }

// Parses the IPC spec via the pure parser (utils/IpcSpecParse) and publishes
// the result only after the full document produced at least one valid method.
bool ParseToml(const std::string& body) {
    std::unordered_map<std::string, std::uint8_t> interfaceIds;
    std::unordered_map<std::string, MethodSpec> methods;
    if (!ParseSpecToml(body, interfaceIds, methods))
        return false;
    g_state.ipcSpec.interfaceIds = std::move(interfaceIds);
    g_state.ipcSpec.methods = std::move(methods);
    return true;
}

bool ReadBody(const std::string& path, std::string& outBody) {
    std::ifstream f(winfile::ToPath(path), std::ios::binary);
    if (!f)
        return false;
    std::ostringstream ss;
    ss << f.rdbuf();
    outBody = ss.str();
    return !outBody.empty();
}

// Reads the spec from the local cache directory.
bool LoadFromCache() {
    std::string body;
    if (!ReadBody(CachePath(), body))
        return false;
    return ParseToml(body);
}

void SetStatus(std::uint64_t version) {
    std::lock_guard lock(g_state.statusMetadataMutex);
    g_state.steamclientIpcPatternVersion = version;
}

// Adopts the published generation when the index says the table moved. The
// candidate is parsed before the cache is replaced, so a broken publish leaves
// the working table in place and the next launch tries again. `applied` is the
// generation the cache records, which decides whether identical bytes mean a
// stale fetch or a cache that never recorded one.
bool Refresh(std::uint64_t remote, std::uint64_t applied) {
    const std::string path = CachePath();
    const std::string candidatePath = path + ".tmp";
    std::string source;
    if (!downloader::Download(kSubdir, g_state.steamclientSha, candidatePath, &source, kRefreshTimeoutSec)) {
        AC_LOG_INFO(kModule, "No source can refresh the IPC spec right now; keeping the cached one.");
        return false;
    }

    if (!pattern::AdoptDownload(true, applied, winfile::FilesEqual(candidatePath, path))) {
        // The index says the spec moved but the endpoint served the bytes a
        // cache with a recorded generation already holds (edge cache): keep the
        // recorded generation so the next launch asks again instead of losing
        // the correction.
        winfile::Delete(candidatePath);
        AC_LOG_WARN(kModule, "Upstream re-served the IPC spec already cached; retrying next launch.");
        return false;
    }

    std::string body;
    std::unordered_map<std::string, std::uint8_t> interfaceIds;
    std::unordered_map<std::string, MethodSpec> methods;
    if (!ReadBody(candidatePath, body) || !ParseSpecToml(body, interfaceIds, methods)) {
        winfile::Delete(candidatePath);
        AC_LOG_WARN(kModule, "Upstream IPC spec candidate could not be parsed; keeping the cached one.");
        return false;
    }
    if (!winfile::MoveReplace(candidatePath, path)) {
        winfile::Delete(candidatePath);
        AC_LOG_WARN(kModule, "Could not adopt the upstream IPC spec; keeping the cached one.");
        return false;
    }

    const std::size_t entries = methods.size();
    g_state.ipcSpec.interfaceIds = std::move(interfaceIds);
    g_state.ipcSpec.methods = std::move(methods);
    if (!pattern::sidecar::Write(path + ".src", source, remote)) {
        AC_LOG_WARN(kModule, "Could not record the IPC spec generation; the next launch will refresh again.");
    }
    AC_LOG_INFO(kModule, "Refreshed IPC spec to generation %llu from '%s' (%zu entries).",
                static_cast<unsigned long long>(remote), source.c_str(), entries);
    return true;
}

} // namespace

bool Init() {
    static std::mutex initMutex; // private initialization lifecycle
    std::lock_guard lock(initMutex);
    if (g_state.ipcSpec.loaded)
        return true; // already done

    if (g_state.steamclientSha.empty() || g_state.steamclientSha.size() != 64) {
        AC_LOG_WARN(kModule, "No steamclient SHA; skipping IPC spec load.");
        return false;
    }

    const std::string path = CachePath();
    const std::string srcPath = path + ".src";
    // 0 when the index is unreachable or silent for this build: the cache then
    // stays exactly as it is.
    const std::uint64_t remote = pattern::RemoteVersion(kSubdir, g_state.steamclientSha);

    // 1. Local cache first. A usable table is served immediately; a newer
    //    published generation is adopted in the same call.
    if (LoadFromCache()) {
        std::uint64_t applied = pattern::sidecar::Read(srcPath).version;
        if (remote > applied && Refresh(remote, applied)) {
            // Report what the sidecar now says: a failed write leaves the old
            // number in place even though the table was adopted.
            applied = pattern::sidecar::Read(srcPath).version;
        }
        SetStatus(applied);
        g_state.ipcSpec.loaded = true;
        AC_LOG_INFO(kModule, "Loaded IPC spec from cache (%zu entries).", g_state.ipcSpec.methods.size());
        return true;
    }

    // 2. Download from the same mirror chain as pattern TOMLs.
    {
        std::error_code ec;
        std::filesystem::create_directories(winfile::ToPath(path).parent_path(), ec);
    }

    std::string source;
    if (!downloader::Download(kSubdir, g_state.steamclientSha, path, &source)) {
        AC_LOG_WARN(kModule,
                    "IPC spec download failed for SHA %s; "
                    "falling back to compile-time hashes.",
                    g_state.steamclientSha.c_str());
        return false;
    }

    // 3. Parse the freshly downloaded file.
    if (!LoadFromCache()) {
        AC_LOG_WARN(kModule, "IPC spec parse failed after download.");
        return false;
    }

    if (!pattern::sidecar::Write(srcPath, source, remote)) {
        AC_LOG_WARN(kModule, "Could not record the IPC spec source; the next launch may refresh again.");
        // Report what the file actually says, not what the index does.
        SetStatus(pattern::sidecar::Read(srcPath).version);
    } else {
        SetStatus(remote);
    }
    g_state.ipcSpec.loaded = true;
    AC_LOG_INFO(kModule, "Loaded IPC spec from %s (%zu entries).", source.c_str(), g_state.ipcSpec.methods.size());
    return true;
}

std::optional<std::uint8_t> ResolveInterfaceId(const char* interfaceName) {
    if (!g_state.ipcSpec.loaded || !interfaceName)
        return std::nullopt;
    auto it = g_state.ipcSpec.interfaceIds.find(interfaceName);
    if (it != g_state.ipcSpec.interfaceIds.end())
        return it->second;
    return std::nullopt;
}

std::optional<std::uint32_t> ResolveHash(const char* qualifiedName) {
    if (!g_state.ipcSpec.loaded || !qualifiedName)
        return std::nullopt;
    auto it = g_state.ipcSpec.methods.find(qualifiedName);
    if (it != g_state.ipcSpec.methods.end())
        return it->second.hash;
    return std::nullopt;
}

std::optional<MethodSpec> ResolveMethodSpec(const char* qualifiedName) {
    if (!g_state.ipcSpec.loaded || !qualifiedName)
        return std::nullopt;
    auto it = g_state.ipcSpec.methods.find(qualifiedName);
    if (it != g_state.ipcSpec.methods.end())
        return it->second;
    return std::nullopt;
}

bool IsLoaded() { return g_state.ipcSpec.loaded; }

} // namespace ac::ipcspec
