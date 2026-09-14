#include "pch.h"
#include "workshop/WorkshopSync.h"

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <exception>
#include <filesystem>
#include <fstream>
#include <map>
#include <mutex>
#include <string>
#include <string_view>
#include <thread>
#include <utility>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "hooks/wire/BackupIo.h"
#include "network/HubcapQuota.h"
#include "network/RuntimeHttp.h"
#include "security/ProviderCredentials.h"
#include "utils/JsonStringField.h"

namespace ac::workshop {
namespace {

namespace fs = std::filesystem;

constexpr const char* kModule = "WorkshopSync";
// Steam writes one ACF per app with Workshop subscriptions:
// steamapps/workshop/appworkshop_<appid>.acf
constexpr std::string_view kWorkshopAcfPrefix = "appworkshop_";
constexpr std::chrono::seconds kPollIntervalSec(15);
constexpr std::chrono::seconds kInitialDelaySec(5);
// SHORT manifest-only retry schedule (seconds). Workshop payload downloads
// stay with Steam; only the small manifest fetch is retried here, so there
// is no reason for the 30s..30min Desk ladder.
constexpr int kMaxAttempts = 6;
constexpr int kBackoffSec[kMaxAttempts] = {10, 15, 30, 60, 120, 300};
constexpr int kFetchTimeoutSec = 6;
constexpr int kUsageProbeTimeoutSec = 2;

std::atomic<bool> s_running{false};
std::thread s_thread;

struct ItemKey {
    std::uint32_t appId = 0;
    std::uint64_t workshopId = 0;
    std::uint64_t manifestGid = 0;

    bool operator<(const ItemKey& o) const {
        if (appId != o.appId) return appId < o.appId;
        if (workshopId != o.workshopId) return workshopId < o.workshopId;
        return manifestGid < o.manifestGid;
    }
};

struct AttemptState {
    int attempts = 0;
    std::chrono::steady_clock::time_point nextEligible{};
};

std::mutex s_stateMutex;
std::map<ItemKey, AttemptState> s_states;

struct WorkshopItem {
    std::uint32_t appId = 0;
    std::uint64_t workshopId = 0;
    std::uint64_t manifestGid = 0;
};

bool IsUsableFile(const fs::path& path) {
    std::error_code ec;
    if (!fs::is_regular_file(path, ec) || ec) return false;
    const auto size = fs::file_size(path, ec);
    return !ec && size > 0;
}

// The manifest filename carries the depot id (<depot>_<gid>.manifest) which
// is unknown until the manifest bytes are inspected, so presence is checked
// by GID suffix across both Steam depotcache locations (mirrors Desk's
// has_local_manifest_gid).
bool HasManifestByGid(std::uint64_t gid) {
    if (gid == 0 || g_state.steamInstallPath.empty()) return false;
    const std::string suffix = "_" + std::to_string(gid) + ".manifest";
    const fs::path steamRoot(g_state.steamInstallPath);
    const fs::path dirs[] = {steamRoot / "depotcache",
                             steamRoot / "config" / "depotcache"};
    std::error_code ec;
    for (const auto& dir : dirs) {
        for (fs::directory_iterator it(dir, ec), end; !ec && it != end;
             it.increment(ec)) {
            const fs::path p = it->path();
            const std::string name = p.filename().string();
            if (name.size() > suffix.size() &&
                name.compare(name.size() - suffix.size(), suffix.size(),
                             suffix) == 0 &&
                IsUsableFile(p)) {
                return true;
            }
        }
    }
    return false;
}

std::vector<std::string> LibrarySteamappsDirs() {
    std::vector<std::string> dirs;
    if (g_state.steamInstallPath.empty()) return dirs;
    const fs::path primary =
        fs::path(g_state.steamInstallPath) / "steamapps";
    dirs.push_back(primary.string());
    // Secondary libraries from libraryfolders.vdf ("path" "<dir>").
    std::error_code ec;
    if (fs::is_regular_file(primary / "libraryfolders.vdf", ec) && !ec) {
        std::ifstream input(primary / "libraryfolders.vdf", std::ios::binary);
        if (input) {
            const std::string content((std::istreambuf_iterator<char>(input)),
                                      std::istreambuf_iterator<char>());
            std::size_t pos = 0;
            while ((pos = content.find("\"path\"", pos)) != std::string::npos) {
                pos += 6;
                const std::size_t open = content.find('"', pos);
                if (open == std::string::npos) break;
                const std::size_t close = content.find('"', open + 1);
                if (close == std::string::npos) break;
                std::string library = content.substr(open + 1, close - open - 1);
                // VDF escapes Windows separators as `\\`.
                for (std::size_t p = 0;
                     (p = library.find("\\\\", p)) != std::string::npos; ++p) {
                    library.replace(p, 2, "\\");
                }
                const std::string steamapps =
                    (fs::path(library) / "steamapps").string();
                if (std::find(dirs.begin(), dirs.end(), steamapps) ==
                    dirs.end()) {
                    dirs.push_back(steamapps);
                }
                pos = close + 1;
            }
        }
    }
    return dirs;
}

std::uint32_t AppIdFromWorkshopAcf(const fs::path& path) {
    // appworkshop_<appid>.acf
    const std::string stem = path.stem().string();
    constexpr std::string_view kPrefix = kWorkshopAcfPrefix;
    if (stem.size() <= kPrefix.size() ||
        stem.compare(0, kPrefix.size(), kPrefix) != 0) {
        return 0;
    }
    std::uint32_t appId = 0;
    for (std::size_t i = kPrefix.size(); i < stem.size(); ++i) {
        if (stem[i] < '0' || stem[i] > '9') return 0;
        appId = appId * 10 + static_cast<std::uint32_t>(stem[i] - '0');
    }
    return appId;
}

// Minimal ACF item scanner: finds "<workshopid>" { ... "manifest" "<gid>" }.
// Bounded forward search keeps a stray "manifest" token from pairing with
// the wrong item.
void ParseWorkshopAcf(const std::string& body, std::uint32_t appId,
                      std::vector<WorkshopItem>& out) {
    std::size_t pos = 0;
    while (pos < body.size()) {
        const std::size_t open = body.find('"', pos);
        if (open == std::string::npos) break;
        const std::size_t close = body.find('"', open + 1);
        if (close == std::string::npos) break;
        const std::string idTok = body.substr(open + 1, close - open - 1);
        std::size_t p = close + 1;
        while (p < body.size() &&
               (body[p] == ' ' || body[p] == '\t' || body[p] == '\r' ||
                body[p] == '\n')) {
            ++p;
        }
        if (p >= body.size() || body[p] != '{') {
            pos = close + 1;
            continue;
        }
        std::uint64_t workshopId = 0;
        for (char c : idTok) {
            if (c < '0' || c > '9') {
                workshopId = 0;
                break;
            }
            workshopId = workshopId * 10 + static_cast<std::uint64_t>(c - '0');
        }
        if (workshopId == 0) {
            pos = close + 1;
            continue;
        }
        // Look for the "manifest" key within a bounded window past the brace.
        const std::size_t windowEnd = std::min(body.size(), p + 2048);
        const std::size_t mKey = body.find("\"manifest\"", p);
        if (mKey == std::string::npos || mKey >= windowEnd) {
            pos = close + 1;
            continue;
        }
        const std::size_t vOpen = body.find('"', mKey + 10);
        if (vOpen == std::string::npos || vOpen >= windowEnd) {
            pos = close + 1;
            continue;
        }
        const std::size_t vClose = body.find('"', vOpen + 1);
        if (vClose == std::string::npos || vClose >= windowEnd) {
            pos = close + 1;
            continue;
        }
        std::uint64_t gid = 0;
        for (std::size_t i = vOpen + 1; i < vClose; ++i) {
            if (body[i] < '0' || body[i] > '9') {
                gid = 0;
                break;
            }
            gid = gid * 10 + static_cast<std::uint64_t>(body[i] - '0');
        }
        if (gid != 0) {
            out.push_back(WorkshopItem{appId, workshopId, gid});
        }
        pos = vClose + 1;
    }
}

std::vector<WorkshopItem> DiscoverWorkshopItems() {
    std::vector<WorkshopItem> items;
    for (const std::string& dir : LibrarySteamappsDirs()) {
        const fs::path workshopDir = fs::path(dir) / "workshop";
        std::error_code ec;
        for (fs::directory_iterator it(workshopDir, ec), end; !ec && it != end;
             it.increment(ec)) {
            const fs::path path = it->path();
            const std::string name = path.filename().string();
            // NOTE: the prefix is 12 chars ("appworkshop_"); comparing 13 would
            // swallow the first appId digit and skip every ACF in the library.
            if (name.compare(0, kWorkshopAcfPrefix.size(), kWorkshopAcfPrefix) != 0 ||
                path.extension() != ".acf") {
                continue;
            }
            const std::uint32_t appId = AppIdFromWorkshopAcf(path);
            if (appId == 0) continue;
            std::ifstream input(path, std::ios::binary);
            if (!input) continue;
            const std::string body((std::istreambuf_iterator<char>(input)),
                                   std::istreambuf_iterator<char>());
            ParseWorkshopAcf(body, appId, items);
        }
    }
    std::sort(items.begin(), items.end(), [](const WorkshopItem& a,
                                             const WorkshopItem& b) {
        if (a.workshopId != b.workshopId) return a.workshopId < b.workshopId;
        return a.manifestGid < b.manifestGid;
    });
    items.erase(std::unique(items.begin(), items.end(),
                            [](const WorkshopItem& a, const WorkshopItem& b) {
                                return a.workshopId == b.workshopId;
                            }),
                items.end());
    return items;
}

// ---------------------------------------------------------------------------
// Generation lane configuration (part 2).
// ---------------------------------------------------------------------------
constexpr const char* kHubcapApi = "https://hubcapmanifest.com/api/v1";
constexpr const wchar_t* kUserAgent = L"AetherCore/HubcapManifest/1.0";

// Per-pass cap on REMOTE generations. Local hits and validated Desk-cache hits
// are unlimited: only provider traffic is rationed, so a depotcache wipe can
// never burn the whole shared daily budget in one startup storm. Deferred items
// keep their attempt counter (a cap is not a failure) and drain on later passes.
constexpr int kMaxGenerationsPerPass = 5;

// Steam manifest section magics (ContentManifestPayload / ContentManifestMetadata).
constexpr std::uint32_t kPayloadMagic = 0x71F617D0u;
constexpr std::uint32_t kMetadataMagic = 0x1F4812BEu;

// ---------------------------------------------------------------------------
// Manifest identity: depot id + GID carried by the manifest bytes themselves.
//
// A Workshop item's depot id is not knowable from appworkshop_<app>.acf, and
// the depotcache filename is <depot>_<gid>.manifest, so the bytes must be
// inspected before staging. Steam manifests are four little-endian framed
// protobuf sections; only ContentManifestMetadata fields 1 (depot_id) and 2
// (gid_manifest) matter here, so a small wire reader keeps this lane free of
// any dependency on the generated protobufs. Mirrors Desk's manifest_identity.
// ---------------------------------------------------------------------------
bool ReadU32Le(const std::string& bytes, std::size_t& offset, std::uint32_t& out) {
    if (bytes.size() < offset || bytes.size() - offset < 4) return false;
    out = static_cast<std::uint32_t>(static_cast<unsigned char>(bytes[offset])) |
          (static_cast<std::uint32_t>(static_cast<unsigned char>(bytes[offset + 1])) << 8) |
          (static_cast<std::uint32_t>(static_cast<unsigned char>(bytes[offset + 2])) << 16) |
          (static_cast<std::uint32_t>(static_cast<unsigned char>(bytes[offset + 3])) << 24);
    offset += 4;
    return true;
}

bool ReadVarint(std::string_view bytes, std::size_t& cursor, std::uint64_t& out) {
    std::uint64_t value = 0;
    for (int shift = 0; shift < 70; shift += 7) {
        if (cursor >= bytes.size()) return false;
        const auto byte = static_cast<unsigned char>(bytes[cursor++]);
        value |= static_cast<std::uint64_t>(byte & 0x7f) << shift;
        if ((byte & 0x80) == 0) {
            out = value;
            return true;
        }
    }
    return false;
}

bool ManifestIdentity(const std::string& body, std::uint32_t& depotId,
                      std::uint64_t& gid) {
    // A ZIP-wrapped reply would need inflate; AetherCore links no zlib and the
    // authenticated generation route answers with raw manifest bytes (the same
    // contract ManifestFetch relies on for depot manifests). A ZIP is therefore
    // rejected loudly instead of being staged as an unusable manifest.
    if (body.size() >= 4 && body.compare(0, 4, "PK" "\x03" "\x04") == 0) {
        AC_LOG_WARN(kModule, "Hubcap returned a ZIP-wrapped Workshop manifest; not supported in-process.");
        return false;
    }
    std::size_t offset = 0;
    std::uint32_t magic = 0;
    std::uint32_t length = 0;
    if (!ReadU32Le(body, offset, magic) || !ReadU32Le(body, offset, length) ||
        magic != kPayloadMagic || body.size() - offset < length) {
        return false;
    }
    offset += length;
    if (!ReadU32Le(body, offset, magic) || !ReadU32Le(body, offset, length) ||
        magic != kMetadataMagic || body.size() - offset < length) {
        return false;
    }

    const std::string_view metadata(body.data() + offset, length);
    std::size_t cursor = 0;
    std::uint32_t depot = 0;
    std::uint64_t manifestGid = 0;
    while (cursor < metadata.size()) {
        std::uint64_t tag = 0;
        if (!ReadVarint(metadata, cursor, tag)) return false;
        const std::uint64_t field = tag >> 3;
        switch (tag & 7) {
        case 0: {
            std::uint64_t value = 0;
            if (!ReadVarint(metadata, cursor, value)) return false;
            if (field == 1) depot = static_cast<std::uint32_t>(value);
            if (field == 2) manifestGid = value;
            break;
        }
        case 1:
            if (metadata.size() - cursor < 8) return false;
            cursor += 8;
            break;
        case 2: {
            std::uint64_t size = 0;
            if (!ReadVarint(metadata, cursor, size) || size > metadata.size() - cursor) return false;
            cursor += static_cast<std::size_t>(size);
            break;
        }
        case 5:
            if (metadata.size() - cursor < 4) return false;
            cursor += 4;
            break;
        default:
            return false;
        }
    }
    if (depot == 0 || manifestGid == 0) return false;
    depotId = depot;
    gid = manifestGid;
    return true;
}

// ---------------------------------------------------------------------------
// File plumbing.
// ---------------------------------------------------------------------------
bool ReadWholeFile(const fs::path& path, std::string& out) {
    std::ifstream input(path, std::ios::binary);
    if (!input) return false;
    out.assign((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
    return !out.empty();
}

// Atomic write (tmp -> replace) plus a post-write size check: a truncated
// manifest in depotcache is worse than no manifest at all, because Steam would
// keep reading the partial file instead of asking for it again.
bool WriteVerified(const fs::path& destination, const std::string& bytes) {
    if (bytes.empty()) return false;
    std::error_code ec;
    if (!fs::create_directories(destination.parent_path(), ec) && ec) {
        AC_LOG_WARN(kModule, "Could not create %s.", destination.parent_path().string().c_str());
        return false;
    }
    const fs::path temporary = destination.string() + ".aether-tmp";
    {
        std::ofstream output(temporary, std::ios::binary | std::ios::trunc);
        if (!output) return false;
        output.write(bytes.data(), static_cast<std::streamsize>(bytes.size()));
        output.close();
        if (!output) {
            fs::remove(temporary, ec);
            return false;
        }
    }
    if (!backup::io::AtomicReplace(temporary.string(), destination.string())) {
        fs::remove(temporary, ec);
        return false;
    }
    const auto installed = fs::file_size(destination, ec);
    if (ec || installed != static_cast<std::uintmax_t>(bytes.size())) {
        AC_LOG_WARN(kModule, "Post-install verification failed for %s.",
                    destination.string().c_str());
        return false;
    }
    return true;
}

// <AetherData>\temp\hubcap\workshop\<workshop_id>.manifest — the Hubcap cache
// AetherDesk writes from its manual Repair Workshop button. Sharing the file
// means a depotcache wipe never spends the shared daily budget twice, and an
// item already generated by Desk is staged locally with no provider traffic.
fs::path WorkshopCachePath(std::uint64_t workshopId) {
    const std::string deskData = backup::io::CachedDeskDataDir();
    if (deskData.empty()) return {};
    return fs::path(deskData) / "temp" / "hubcap" / "workshop" /
           (std::to_string(workshopId) + ".manifest");
}

// Returns the cached manifest only when it parses AND carries the GID the ACF
// asks for; a stale or corrupt cache is ignored (and logged) so the lane falls
// through to an authenticated generation.
bool LoadValidCachedManifest(std::uint64_t workshopId, std::uint64_t expectedGid,
                             std::uint32_t& depotId, std::string& bytes) {
    const fs::path cachePath = WorkshopCachePath(workshopId);
    if (cachePath.empty()) return false;
    std::error_code ec;
    if (!fs::is_regular_file(cachePath, ec) || ec) return false;

    std::string body;
    if (!ReadWholeFile(cachePath, body)) {
        AC_LOG_WARN(kModule, "Workshop cache read failed item=%llu path=%s.",
                    static_cast<unsigned long long>(workshopId), cachePath.string().c_str());
        return false;
    }
    std::uint32_t depot = 0;
    std::uint64_t gid = 0;
    if (!ManifestIdentity(body, depot, gid)) {
        AC_LOG_WARN(kModule, "Ignoring invalid Workshop cache item=%llu.",
                    static_cast<unsigned long long>(workshopId));
        return false;
    }
    if (gid != expectedGid) {
        AC_LOG_DEBUG(kModule,
                     "Ignoring stale Workshop cache item=%llu cached_gid=%llu expected_gid=%llu.",
                     static_cast<unsigned long long>(workshopId),
                     static_cast<unsigned long long>(gid),
                     static_cast<unsigned long long>(expectedGid));
        return false;
    }
    depotId = depot;
    bytes.swap(body);
    return true;
}

// Stages <depot>_<gid>.manifest into Steam\depotcache. Steam's own Workshop
// retry picks the file up on its next pass: no client restart, no steam://
// spawn, nothing injected back into the running client.
bool InstallToDepotcache(std::uint32_t depotId, std::uint64_t gid,
                         const std::string& bytes) {
    if (g_state.steamInstallPath.empty()) return false;
    const fs::path destination =
        fs::path(g_state.steamInstallPath) / "depotcache" /
        (std::to_string(depotId) + "_" + std::to_string(gid) + ".manifest");

    std::error_code ec;
    if (IsUsableFile(destination) &&
        fs::file_size(destination, ec) == static_cast<std::uintmax_t>(bytes.size()) && !ec) {
        AC_LOG_DEBUG(kModule, "Workshop manifest already staged depot=%u gid=%llu.", depotId,
                     static_cast<unsigned long long>(gid));
        return true;
    }
    if (!WriteVerified(destination, bytes)) {
        AC_LOG_WARN(kModule, "Could not install Workshop manifest depot=%u gid=%llu.", depotId,
                    static_cast<unsigned long long>(gid));
        return false;
    }
    AC_LOG_INFO(kModule, "Workshop manifest installed depot=%u gid=%llu bytes=%llu.", depotId,
                static_cast<unsigned long long>(gid),
                static_cast<unsigned long long>(bytes.size()));
    return true;
}

// Mirrors Desk's backup_workshop_manifest: <AetherData>\backup\<app>\lua\<depot>_<gid>.manifest.
// No Lua pin ever references a Workshop depot, so neither the startup backup
// nor ManifestRestore::BackupManifestAfterGeneration can capture these files;
// the uninstall/restore contract needs this explicit copy. Best-effort, and it
// never overwrites a usable destination (shared backup contract).
void BackupManifest(std::uint32_t appId, std::uint32_t depotId, std::uint64_t gid,
                    const std::string& bytes) {
    const std::string deskData = backup::io::CachedDeskDataDir();
    if (deskData.empty()) return;
    const fs::path destination =
        fs::path(deskData) / "backup" / std::to_string(appId) / "lua" /
        (std::to_string(depotId) + "_" + std::to_string(gid) + ".manifest");
    if (IsUsableFile(destination)) return;
    if (WriteVerified(destination, bytes)) {
        AC_LOG_INFO(kModule,
                    "Workshop manifest archived in AetherData backup app=%u depot=%u gid=%llu bytes=%llu.",
                    appId, depotId, static_cast<unsigned long long>(gid),
                    static_cast<unsigned long long>(bytes.size()));
    }
}

// A staged manifest is CDN metadata, not the Workshop payload. When the
// manifest is MISSING the partial content dir is what makes Steam resume
// corrupt chunks (the "50kbps then no connection" stall), so dropping it lets
// the client re-download cleanly. This runs ONLY on the missing-manifest
// branch: with a manifest present the folder is never touched.
void RemoveStaleContentDirs(std::uint32_t appId, std::uint64_t workshopId) {
    for (const std::string& steamapps : LibrarySteamappsDirs()) {
        const fs::path content = fs::path(steamapps) / "workshop" / "content" /
                                 std::to_string(appId) / std::to_string(workshopId);
        std::error_code ec;
        if (!fs::is_directory(content, ec) || ec) continue;
        const auto removed = fs::remove_all(content, ec);
        if (ec) {
            AC_LOG_WARN(kModule, "Could not remove stale Workshop content dir app=%u item=%llu: %s.",
                        appId, static_cast<unsigned long long>(workshopId), ec.message().c_str());
        } else {
            AC_LOG_INFO(kModule, "Removed stale Workshop content dir app=%u item=%llu entries=%llu.",
                        appId, static_cast<unsigned long long>(workshopId),
                        static_cast<unsigned long long>(removed));
        }
    }
}

// ---------------------------------------------------------------------------
// Authenticated Hubcap lane.
// ---------------------------------------------------------------------------

// Mirrors ManifestFetch's usage probe but reads the "workshop" bucket: a
// definitive "no" stops the pass before any shared budget is spent, while a
// network/parse failure fails open (the probe runs on this background thread
// and the server enforces its own limits anyway).
bool HubcapWorkshopAllowed(const std::string& apiKey) {
    const http::Response response = http::GetUncheckedWithHeaders(
        std::string(kHubcapApi) + "/generate/usage", kUsageProbeTimeoutSec,
        {"Authorization: Bearer " + apiKey}, kUserAgent);
    if (response.networkError || response.status != 200) {
        AC_LOG_DEBUG(kModule,
                     "Workshop generation usage probe unavailable (network=%d HTTP=%d); proceeding.",
                     response.networkError ? 1 : 0, response.status);
        return true;
    }
    bool serviceReady = true;
    if (jsonutil::PullBoolField(response.body, "steam_service_ready", serviceReady) &&
        !serviceReady) {
        AC_LOG_WARN(kModule, "Hubcap reports the Steam generation service is not ready.");
        return false;
    }
    // One bucket per kind (single/bundle/workshop); only "workshop" applies here.
    const std::size_t bucket = response.body.find("\"workshop\"");
    if (bucket != std::string::npos) {
        const std::size_t open = response.body.find('{', bucket);
        const std::size_t close = response.body.find('}', bucket);
        if (open != std::string::npos && close != std::string::npos && close > open) {
            const std::string_view workshop(response.body.data() + open, close - open);
            std::uint64_t remaining = 0;
            if (jsonutil::PullUIntField(workshop, "remaining", remaining) && remaining == 0) {
                AC_LOG_WARN(kModule, "Hubcap Workshop-manifest generation quota is exhausted.");
                return false;
            }
        }
    }
    return true;
}

// Generates one Workshop item manifest. The shared daily budget
// (<AetherData>\state\hubcap_generation_quota.json, 500/day, spent together
// with Desk's manual button) is reserved before the request and given back on
// every failure path, so a transient error never burns a unit.
bool GenerateManifest(std::uint64_t workshopId, std::uint64_t expectedGid,
                      std::uint32_t& depotId, std::string& bytes) {
    const auto apiKey = security::ReadHubcapApiKey();
    if (!apiKey) {
        AC_LOG_DEBUG(kModule, "Hubcap skipped: encrypted provider credentials unavailable.");
        return false;
    }
    if (!HubcapWorkshopAllowed(*apiKey)) return false;
    if (!hubcapquota::TryReserveWorkshopGeneration()) return false;

    const std::string url =
        std::string(kHubcapApi) + "/generate/workshopmanifest/" + std::to_string(workshopId);
    const std::vector<std::string> headers = {"Authorization: Bearer " + *apiKey};
    const http::Response response =
        http::GetUncheckedWithHeaders(url, kFetchTimeoutSec, headers, kUserAgent);

    if (response.networkError) {
        hubcapquota::ReleaseWorkshopGeneration();
        AC_LOG_WARN(kModule, "Workshop manifest request failed for item=%llu (network).",
                    static_cast<unsigned long long>(workshopId));
        return false;
    }
    if (response.status != 200 || response.body.empty()) {
        hubcapquota::ReleaseWorkshopGeneration();
        AC_LOG_WARN(kModule, "Workshop manifest request HTTP=%d for item=%llu.", response.status,
                    static_cast<unsigned long long>(workshopId));
        return false;
    }
    std::uint32_t depot = 0;
    std::uint64_t gid = 0;
    if (!ManifestIdentity(response.body, depot, gid)) {
        hubcapquota::ReleaseWorkshopGeneration();
        AC_LOG_WARN(kModule, "Hubcap Workshop manifest for item=%llu is not a valid manifest.",
                    static_cast<unsigned long long>(workshopId));
        return false;
    }
    if (gid != expectedGid) {
        hubcapquota::ReleaseWorkshopGeneration();
        AC_LOG_WARN(kModule,
                    "Hubcap Workshop manifest mismatch for item=%llu expected_gid=%llu received_gid=%llu.",
                    static_cast<unsigned long long>(workshopId),
                    static_cast<unsigned long long>(expectedGid),
                    static_cast<unsigned long long>(gid));
        return false;
    }
    AC_LOG_DEBUG(kModule, "Hubcap Workshop manifest validated item=%llu depot=%u gid=%llu bytes=%llu.",
                 static_cast<unsigned long long>(workshopId), depot,
                 static_cast<unsigned long long>(gid),
                 static_cast<unsigned long long>(response.body.size()));
    depotId = depot;
    bytes = response.body;
    return true;
}

// ---------------------------------------------------------------------------
// Per-item sync + polling pass.
// ---------------------------------------------------------------------------
enum class ItemOutcome {
    kDone,      // manifest is available in depotcache (was already, or just staged)
    kRetry,     // real failure: schedule with the short backoff ladder
    kDeferred,  // per-pass generation cap: run again soon, attempts untouched
};

ItemOutcome SyncItem(const WorkshopItem& item, int& generationBudget) {
    if (HasManifestByGid(item.manifestGid)) {
        AC_LOG_DEBUG(kModule, "Workshop manifest already local item=%llu gid=%llu.",
                     static_cast<unsigned long long>(item.workshopId),
                     static_cast<unsigned long long>(item.manifestGid));
        return ItemOutcome::kDone;
    }

    // Missing manifest: clear the partial payload first, so Steam re-downloads
    // cleanly once the manifest lands.
    RemoveStaleContentDirs(item.appId, item.workshopId);

    std::uint32_t depotId = 0;
    std::string bytes;
    const bool fromCache =
        LoadValidCachedManifest(item.workshopId, item.manifestGid, depotId, bytes);
    if (!fromCache) {
        if (generationBudget <= 0) {
            AC_LOG_DEBUG(kModule,
                         "Workshop generation budget spent for this pass; item=%llu deferred.",
                         static_cast<unsigned long long>(item.workshopId));
            return ItemOutcome::kDeferred;
        }
        --generationBudget;
        if (!GenerateManifest(item.workshopId, item.manifestGid, depotId, bytes)) {
            return ItemOutcome::kRetry;
        }
        // Best-effort: the Desk cache is a budget saver, not a requirement.
        const fs::path cachePath = WorkshopCachePath(item.workshopId);
        if (!cachePath.empty() && !WriteVerified(cachePath, bytes)) {
            AC_LOG_WARN(kModule, "Could not refresh the Desk Workshop cache for item=%llu.",
                        static_cast<unsigned long long>(item.workshopId));
        }
    }

    if (!InstallToDepotcache(depotId, item.manifestGid, bytes)) {
        // The generation itself succeeded, but nothing usable was staged: give
        // the reserved unit back, exactly like the depot-manifest lane does.
        if (!fromCache) hubcapquota::ReleaseWorkshopGeneration();
        return ItemOutcome::kRetry;
    }
    BackupManifest(item.appId, depotId, item.manifestGid, bytes);
    AC_LOG_INFO(kModule, "Workshop manifest staged origin=%s app=%u item=%llu depot=%u gid=%llu.",
                fromCache ? "cache" : "generated", item.appId,
                static_cast<unsigned long long>(item.workshopId), depotId,
                static_cast<unsigned long long>(item.manifestGid));
    return ItemOutcome::kDone;
}

std::chrono::seconds BackoffFor(int attempts) {
    const int index = std::clamp(attempts, 0, kMaxAttempts - 1);
    return std::chrono::seconds(kBackoffSec[index]);
}

void RunPass() {
    if (g_state.steamInstallPath.empty()) {
        AC_LOG_DEBUG(kModule, "Workshop sync idle: Steam path not resolved yet.");
        return;
    }

    const std::vector<WorkshopItem> items = DiscoverWorkshopItems();
    if (items.empty()) {
        std::lock_guard<std::mutex> lock(s_stateMutex);
        s_states.clear();
        return;
    }

    int generationBudget = kMaxGenerationsPerPass;
    std::size_t resolved = 0;
    std::size_t deferred = 0;
    std::size_t failed = 0;

    for (const WorkshopItem& item : items) {
        if (!s_running.load()) return;
        const ItemKey key{item.appId, item.workshopId, item.manifestGid};

        {
            // SHORT manifest-only ladder: an item still inside its backoff
            // window is skipped without any disk or network work.
            std::lock_guard<std::mutex> lock(s_stateMutex);
            const auto state = s_states.find(key);
            if (state != s_states.end() &&
                state->second.nextEligible > std::chrono::steady_clock::now()) {
                continue;
            }
        }

        const ItemOutcome outcome = SyncItem(item, generationBudget);

        std::lock_guard<std::mutex> lock(s_stateMutex);
        if (outcome == ItemOutcome::kDone) {
            s_states.erase(key);
            ++resolved;
            continue;
        }
        if (outcome == ItemOutcome::kDeferred) {
            // Not a failure: keep the attempt counter and its ladder intact.
            ++deferred;
            continue;
        }

        ++failed;
        AttemptState& state = s_states[key];
        if (state.nextEligible == std::chrono::steady_clock::time_point::max()) continue;
        if (state.attempts >= kMaxAttempts) continue;
        ++state.attempts;
        if (state.attempts >= kMaxAttempts) {
            AC_LOG_WARN(kModule,
                        "Workshop item=%llu gid=%llu gave up after %d attempts this session; "
                        "AetherDesk's Repair Workshop button remains available.",
                        static_cast<unsigned long long>(item.workshopId),
                        static_cast<unsigned long long>(item.manifestGid), state.attempts);
            state.nextEligible = std::chrono::steady_clock::time_point::max();
        } else {
            state.nextEligible = std::chrono::steady_clock::now() + BackoffFor(state.attempts - 1);
        }
    }

    {
        // Forget the attempt history of items that left the subscription set,
        // so a re-subscribe starts with a fresh ladder.
        std::lock_guard<std::mutex> lock(s_stateMutex);
        for (auto it = s_states.begin(); it != s_states.end();) {
            const ItemKey& key = it->first;
            const bool known = std::any_of(
                items.begin(), items.end(), [&key](const WorkshopItem& candidate) {
                    return candidate.appId == key.appId &&
                           candidate.workshopId == key.workshopId &&
                           candidate.manifestGid == key.manifestGid;
                });
            it = known ? std::next(it) : s_states.erase(it);
        }
    }

    if (resolved != 0 || deferred != 0 || failed != 0) {
        AC_LOG_INFO(kModule,
                    "Workshop pass complete discovered=%llu resolved=%llu deferred=%llu failed=%llu.",
                    static_cast<unsigned long long>(items.size()),
                    static_cast<unsigned long long>(resolved),
                    static_cast<unsigned long long>(deferred),
                    static_cast<unsigned long long>(failed));
    }
}

// Interruptible sleep: Stop() must never wait a whole poll interval.
bool SleepWhileRunning(std::chrono::seconds duration) {
    constexpr auto kSlice = std::chrono::milliseconds(200);
    auto remaining = std::chrono::duration_cast<std::chrono::milliseconds>(duration);
    while (remaining.count() > 0 && s_running.load()) {
        const auto slice = std::min<std::chrono::milliseconds>(remaining, kSlice);
        std::this_thread::sleep_for(slice);
        remaining -= slice;
    }
    return s_running.load();
}

void ThreadMain() {
    AC_LOG_INFO(kModule, "Workshop sync thread started (initial delay=%llds, poll=%llds).",
                static_cast<long long>(kInitialDelaySec.count()),
                static_cast<long long>(kPollIntervalSec.count()));
    try {
        // The first pass waits out the init burst: Steam is still writing its
        // appworkshop ACFs while the client boots.
        SleepWhileRunning(kInitialDelaySec);
        while (s_running.load()) {
            RunPass();
            if (!SleepWhileRunning(kPollIntervalSec)) break;
        }
    } catch (const std::exception& ex) {
        AC_LOG_ERROR(kModule, "Workshop sync thread stopped on exception: %s.", ex.what());
    } catch (...) {
        AC_LOG_ERROR(kModule, "Workshop sync thread stopped on an unknown exception.");
    }
    AC_LOG_INFO(kModule, "Workshop sync thread stopped.");
}

}  // namespace

void Start() {
    if (s_running.exchange(true)) {
        AC_LOG_WARN(kModule, "Workshop sync already running.");
        return;
    }
    try {
        s_thread = std::thread(ThreadMain);
    } catch (...) {
        s_running.store(false);
        AC_LOG_ERROR(kModule, "Could not start the Workshop sync thread.");
        return;
    }
    AC_LOG_INFO(kModule,
                "Workshop manifest sync enabled (AetherDLL is the default owner; "
                "AetherDesk's automatic lane is disabled).");
}

void Stop() {
    if (!s_running.exchange(false)) return;
    if (s_thread.joinable()) s_thread.join();
}

}  // namespace ac::workshop
