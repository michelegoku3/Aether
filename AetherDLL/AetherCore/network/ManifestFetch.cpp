#include "pch.h"
#include "core/Workers.h"
#include "network/ManifestFetch.h"

#include <algorithm>
#include <chrono>
#include <cstddef>
#include <cctype>
#include <condition_variable>
#include <deque>
#include <exception>
#include <filesystem>
#include <fstream>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <string_view>
#include <thread>
#include <unordered_map>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "hooks/wire/BackupIo.h"
#include "hooks/wire/ManifestRestore.h"
#include "network/HubcapQuota.h"
#include "network/RuntimeHttp.h"
#include "security/ProviderCredentials.h"
#include "utils/JsonStringField.h"

namespace ac::manifestfetch {
namespace {

constexpr const char* kModule = "ManifestFetch";

struct LookupKey {
    std::uint64_t gid = 0;
    std::uint32_t appId = 0;
    std::uint32_t depotId = 0;

    bool operator==(const LookupKey&) const = default;
};

struct LookupKeyHash {
    std::size_t operator()(const LookupKey& key) const noexcept {
        return static_cast<std::size_t>(key.gid) ^
               (static_cast<std::size_t>(key.gid >> 32) << 1) ^
               (static_cast<std::size_t>(key.appId) << 2) ^
               (static_cast<std::size_t>(key.depotId) << 3);
    }
};

// ---------------------------------------------------------------------------
// Indice locale dei manifest (hot path di BuildDepotDependency: ~1000+ eventi
// a sessione). Il walk della cartella backup/<app>/lua era ripetuto a ogni
// evento; qui lo memorizziamo:
//   * voce POSITIVA: path noto, riverificato con uno stat a ogni hit;
//   * voce NEGATIVA con TTL: niente walk per 30 s (i file nuovi — restore,
//     generazione Hubcap — vengono comunque scoperti alla scadenza, stessa
//     latenza del backoff già esistente).
// Stato privato del modulo (cache di lookup, non dominio: stesso statuto dei
// buffer g_proactive* qui sopra).
// ---------------------------------------------------------------------------
struct LocalManifestCacheEntry {
    std::filesystem::path path;   // valida solo per le voci positive
    bool positive = false;
    std::chrono::steady_clock::time_point expires{};
};
std::mutex g_localManifestMutex;
std::unordered_map<std::string, LocalManifestCacheEntry> g_localManifestCache;
constexpr auto kNegativeTtl = std::chrono::seconds(30);

// A manifest already present on disk is authoritative: Steam's depotcache (or
// the per-game AetherData backup published into depotcache) satisfies the
// dependency without any request-code round-trip. The legacy
// GetManifestRequestCode wire-bridge was removed (see ManifestFetch.h); this
// local-first lookup is now the primary acquisition path.
std::optional<std::filesystem::path> FindLocalManifest(std::uint64_t gid,
                                                        std::uint32_t depotId) {
    namespace fs = std::filesystem;
    const std::string filename = std::to_string(depotId) + "_" +
                                  std::to_string(gid) + ".manifest";
    const fs::path steamRoot(g_state.steamInstallPath);
    const std::vector<fs::path> directCandidates = {
        steamRoot / "depotcache" / filename,
        steamRoot / "config" / "depotcache" / filename,
    };

    auto usable = [](const fs::path& path) -> bool {
        std::error_code ec;
        if (!fs::is_regular_file(path, ec) || ec) return false;
        const auto size = fs::file_size(path, ec);
        return !ec && size > 0;
    };

    for (const fs::path& candidate : directCandidates) {
        if (usable(candidate)) return candidate;
    }

    // Indice: hit positivo (riverificato), hit negativo dentro il TTL = niente walk.
    const auto now = std::chrono::steady_clock::now();
    {
        std::lock_guard<std::mutex> lock(g_localManifestMutex);
        auto it = g_localManifestCache.find(filename);
        if (it != g_localManifestCache.end()) {
            if (it->second.positive) {
                if (usable(it->second.path)) return it->second.path;
                g_localManifestCache.erase(it);   // file sparito: ricalcola
            } else if (now < it->second.expires) {
                return std::nullopt;              // miss recente: salta il walk
            } else {
                g_localManifestCache.erase(it);
            }
        }
    }

    // The targeted ACF-removal restore may be racing with this request, or the
    // DLL may be running before its restore worker has copied the file back.
    // Consult the per-game AetherData backup as a second local source and
    // publish that exact file into Steam/depotcache before claiming a hit.
    const std::string deskData = backup::io::CachedDeskDataDir();
    if (!deskData.empty()) {
        const fs::path backupRoot = fs::path(deskData) / "backup";
        std::error_code ec;
        for (fs::directory_iterator it(backupRoot, ec), end; !ec && it != end; it.increment(ec)) {
            if (!it->is_directory(ec) || ec) continue;
            const fs::path candidate = it->path() / "lua" / filename;
            if (!usable(candidate)) continue;

            const fs::path destination = steamRoot / "depotcache" / filename;
            fs::create_directories(destination.parent_path(), ec);
            if (ec) continue;
            fs::copy_file(candidate, destination,
                          fs::copy_options::overwrite_existing, ec);
            if (!ec && usable(destination)) {
                std::lock_guard<std::mutex> lock(g_localManifestMutex);
                g_localManifestCache[filename] =
                    LocalManifestCacheEntry{destination, true, {}};
                AC_LOG_DEBUG(kModule,
                             "Local manifest index: cached positive entry for %s.",
                             filename.c_str());
                return destination;
            }
            AC_LOG_WARN(kModule,
                        "Local backup manifest found but could not be published to %s (%s).",
                        destination.string().c_str(), ec.message().c_str());
        }
    }
    {
        std::lock_guard<std::mutex> lock(g_localManifestMutex);
        g_localManifestCache[filename] =
            LocalManifestCacheEntry{{}, false, now + kNegativeTtl};
    }
    AC_LOG_DEBUG(kModule,
                 "Local manifest index: %s not on disk; skipping backup walk for %lld s.",
                 filename.c_str(), static_cast<long long>(kNegativeTtl.count()));
    return std::nullopt;
}

bool ReadU32Le(std::string_view bytes, std::size_t& offset, std::uint32_t& out) {
    if (bytes.size() - std::min(offset, bytes.size()) < 4) return false;
    const auto* p = reinterpret_cast<const unsigned char*>(bytes.data() + offset);
    out = static_cast<std::uint32_t>(p[0]) |
          (static_cast<std::uint32_t>(p[1]) << 8) |
          (static_cast<std::uint32_t>(p[2]) << 16) |
          (static_cast<std::uint32_t>(p[3]) << 24);
    offset += 4;
    return true;
}

bool ReadVarint(std::string_view bytes, std::size_t& offset, std::uint64_t& out) {
    out = 0;
    for (unsigned shift = 0; shift < 70; shift += 7) {
        if (offset >= bytes.size()) return false;
        const auto byte = static_cast<unsigned char>(bytes[offset++]);
        out |= static_cast<std::uint64_t>(byte & 0x7f) << shift;
        if ((byte & 0x80) == 0) return true;
    }
    return false;
}

bool ManifestIdentityMatches(std::string_view bytes, std::uint32_t expectedDepot,
                             std::uint64_t expectedGid) {
    std::size_t offset = 0;
    std::uint32_t magic = 0;
    std::uint32_t length = 0;
    if (!ReadU32Le(bytes, offset, magic) || !ReadU32Le(bytes, offset, length) ||
        magic != 0x71F617D0u || bytes.size() - offset < length) return false;
    offset += length;
    if (!ReadU32Le(bytes, offset, magic) || !ReadU32Le(bytes, offset, length) ||
        magic != 0x1F4812BEu || bytes.size() - offset < length) return false;

    const std::string_view metadata = bytes.substr(offset, length);
    std::size_t cursor = 0;
    std::uint32_t depot = 0;
    std::uint64_t gid = 0;
    while (cursor < metadata.size()) {
        std::uint64_t tag = 0;
        if (!ReadVarint(metadata, cursor, tag)) return false;
        const std::uint64_t field = tag >> 3;
        switch (tag & 7) {
        case 0: {
            std::uint64_t value = 0;
            if (!ReadVarint(metadata, cursor, value)) return false;
            if (field == 1) depot = static_cast<std::uint32_t>(value);
            if (field == 2) gid = value;
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
    return depot == expectedDepot && gid == expectedGid;
}

// Mirrors AetherDesk's ensure_generation_available: asks the provider whether
// the authenticated account may generate right now. A definitive "no" fails
// the lookup before any shared budget is spent; a network/parse failure only
// skips the check — the probe runs on the background worker and fails open;
// the server enforces its own limits anyway.
bool HubcapGenerationAllowed(const std::string& apiKey) {
    // Tight budget: the probe precedes the generation on the serialized
    // worker, so a hung connection must not stall the queue.
    constexpr int kUsageProbeTimeoutSec = 2;
    const http::Response response = http::GetUncheckedWithHeaders(
        "https://hubcapmanifest.com/api/v1/generate/usage", kUsageProbeTimeoutSec,
        {"Authorization: Bearer " + apiKey}, L"AetherCore/HubcapManifest/1.0");
    if (response.networkError || response.status != 200) {
        AC_LOG_DEBUG(kModule, "Generation usage probe unavailable (network=%d HTTP=%d); proceeding.",
                     response.networkError ? 1 : 0, response.status);
        return true;
    }
    bool serviceReady = true;
    if (ac::jsonutil::PullBoolField(response.body, "steam_service_ready", serviceReady) &&
        !serviceReady) {
        AC_LOG_WARN(kModule, "Hubcap reports the Steam generation service is not ready.");
        return false;
    }
    // The payload carries one bucket per kind (single/bundle/workshop); only
    // the "single" bucket applies to depot-manifest lookups.
    const std::size_t bucket = response.body.find("\"single\"");
    if (bucket != std::string::npos) {
        const std::size_t open = response.body.find('{', bucket);
        const std::size_t close = response.body.find('}', bucket);
        if (open != std::string::npos && close != std::string::npos && close > open) {
            const std::string_view single(response.body.data() + open, close - open);
            std::uint64_t remaining = 0;
            if (ac::jsonutil::PullUIntField(single, "remaining", remaining) && remaining == 0) {
                AC_LOG_WARN(kModule, "Hubcap single-manifest generation quota is exhausted.");
                return false;
            }
        }
    }
    return true;
}

// Performs the authenticated generation request and atomically installs the
// verified manifest into depotcache. Runs on the background worker only (never
// on the wire thread) with a dedicated, short per-attempt budget — observed
// live generations complete in ~1 s — so a hung connection cannot monopolize
// the serialized queue. Persistence across failures comes from the proactive
// backoff schedule, not from long in-pass retries.
bool FetchAndInstallManifest(const std::string& url,
                             const std::vector<std::string>& headers,
                             std::uint64_t gid, std::uint32_t depotId) {
    constexpr int kGenerationAttempts = 2;
    constexpr int kGenerationAttemptTimeoutSec = 6;
    for (int attempt = 0; attempt < kGenerationAttempts; ++attempt) {
        const http::Response response = http::GetUncheckedWithHeaders(
            url, kGenerationAttemptTimeoutSec, headers, L"AetherCore/HubcapManifest/1.0");
        if (response.networkError) {
            AC_LOG_WARN(kModule, "Hubcap manifest request failed for depot=%u gid=%llu (network).",
                        depotId, static_cast<unsigned long long>(gid));
        } else if (response.status == 200 && !response.body.empty()) {
            if (!ManifestIdentityMatches(response.body, depotId, gid)) {
                AC_LOG_WARN(kModule, "Hubcap manifest identity mismatch for depot=%u gid=%llu.",
                            depotId, static_cast<unsigned long long>(gid));
                return false;
            }

            // Paths are built with fs::path (no manual separator strings);
            // the commit reuses the shared atomic replace helper.
            namespace fs = std::filesystem;
            const fs::path destination = fs::path(g_state.steamInstallPath) / "depotcache" /
                                         (std::to_string(depotId) + "_" +
                                          std::to_string(gid) + ".manifest");
            const fs::path temporary = destination.string() + ".aether-tmp";
            std::error_code dirEc;
            if (!fs::create_directories(destination.parent_path(), dirEc) && dirEc) {
                AC_LOG_WARN(kModule, "Could not create Steam depotcache for Hubcap manifest.");
                return false;
            }
            std::ofstream output(temporary, std::ios::binary | std::ios::trunc);
            output.write(response.body.data(), static_cast<std::streamsize>(response.body.size()));
            output.close();
            if (!output ||
                !backup::io::AtomicReplace(temporary.string(), destination.string())) {
                std::error_code cleanupEc;
                fs::remove(temporary, cleanupEc);
                AC_LOG_WARN(kModule, "Could not atomically install Hubcap manifest depot=%u gid=%llu.",
                            depotId, static_cast<unsigned long long>(gid));
                return false;
            }
            std::error_code sizeEc;
            const auto installedSize = fs::file_size(destination, sizeEc);
            if (sizeEc || installedSize != response.body.size()) {
                AC_LOG_WARN(kModule, "Hubcap manifest post-install verification failed.");
                return false;
            }
            AC_LOG_INFO(kModule, "Hubcap manifest installed depot=%u gid=%llu bytes=%llu.",
                        depotId, static_cast<unsigned long long>(gid),
                        static_cast<unsigned long long>(response.body.size()));
            // Archivia subito il manifest generato nei backup AetherData delle
            // app che lo referenziano: deterministicamente, anche con
            // AetherDesk chiuso (il backup di startup girerebbe solo al
            // prossimo avvio di Steam). Best-effort per contratto.
            ac::hooks::ManifestRestore::BackupManifestAfterGeneration(depotId, gid);
            return true;
        } else if (!response.networkError) {
            AC_LOG_WARN(kModule, "Hubcap manifest request HTTP=%d for depot=%u gid=%llu.",
                        response.status, depotId, static_cast<unsigned long long>(gid));
            if (response.status != 408 && response.status != 425 && response.status != 429 &&
                response.status < 500) return false;
        }
        if (attempt + 1 < kGenerationAttempts) std::this_thread::sleep_for(std::chrono::milliseconds(750));
    }
    return false;
}

bool InstallHubcapManifest(std::uint64_t gid, std::uint32_t depotId) {
    const auto apiKey = security::ReadHubcapApiKey();
    if (!apiKey) {
        AC_LOG_DEBUG(kModule, "Hubcap skipped: encrypted provider credentials unavailable.");
        return false;
    }
    if (!HubcapGenerationAllowed(*apiKey)) return false;
    // The daily generation budget is shared with AetherDesk through
    // <AetherData>\state\hubcap_generation_quota.json; a failed request gives
    // the reserved unit back so a transient failure never burns it.
    if (!hubcapquota::TryReserveGameGeneration()) return false;

    const std::string url = "https://hubcapmanifest.com/api/v1/generate/manifest?depot_id=" +
                            std::to_string(depotId) + "&manifest_id=" + std::to_string(gid);
    const std::vector<std::string> headers = {"Authorization: Bearer " + *apiKey};
    const bool installed = FetchAndInstallManifest(url, headers, gid, depotId);
    if (!installed) {
        hubcapquota::ReleaseGameGeneration();
    }
    return installed;
}

// --- Proactive manifest pipeline (dependency-build trigger) -----------------
// Steam assembles its depot dependency list before every install, update and
// verify pass. That moment is the earliest build-independent point where the
// DLL knows exactly which depot/GID Steam is about to need, so it doubles as
// the default acquisition trigger: local first, then one serialized Hubcap
// generation at a time (mirrors the Desk-side scheduler philosophy and
// protects the shared daily quota from parallel request storms).
constexpr std::size_t kMaxTrackedProactiveKeys = 4096;
// Failed fetches are retried in the background with this backoff schedule
// instead of the old one-shot-per-session: the dependency hook refires while
// Steam downloads/retries and every requeue is gated by nextEligible, so a
// transiently unavailable manifest (Hubcap lag/down/quota) is reattempted
// automatically without request storms.
constexpr int kMaxProactiveRetries = 6;
constexpr int kProactiveBackoffSec[kMaxProactiveRetries] = {30, 60, 120, 300, 600, 900};

struct ProactiveState {
    int attempts = 0;
    std::chrono::steady_clock::time_point nextEligible{};
};

std::mutex g_proactiveMutex;
std::condition_variable g_proactiveCv;
std::deque<LookupKey> g_proactiveQueue;
std::unordered_map<LookupKey, ProactiveState, LookupKeyHash> g_proactiveStates;
bool g_proactiveWorkerStarted = false;

void ProactiveWorkerLoop(std::atomic<bool>& stop) {
    while (!stop.load()) {
        LookupKey key{};
        {
            std::unique_lock<std::mutex> lock(g_proactiveMutex);
            g_proactiveCv.wait(lock, [&] { return stop.load() || !g_proactiveQueue.empty(); });
            if (stop.load()) return;
            key = g_proactiveQueue.front();
            g_proactiveQueue.pop_front();
        }
        try {
            // Another path (ManifestRestore, AetherDesk) may have installed
            // the manifest meanwhile: re-check before spending shared quota.
            if (HasLocalManifest(key.gid, key.depotId)) {
                std::lock_guard<std::mutex> lock(g_proactiveMutex);
                g_proactiveStates.erase(key);
                continue;
            }
            if (InstallHubcapManifest(key.gid, key.depotId)) {
                AC_LOG_INFO(kModule, "Proactive manifest ready: depot=%u gid=%llu.",
                            key.depotId, static_cast<unsigned long long>(key.gid));
                std::lock_guard<std::mutex> lock(g_proactiveMutex);
                g_proactiveStates.erase(key);
            } else {
                int attempts = 0;
                {
                    std::lock_guard<std::mutex> lock(g_proactiveMutex);
                    if (auto it = g_proactiveStates.find(key); it != g_proactiveStates.end()) {
                        attempts = it->second.attempts;
                    }
                }
                AC_LOG_WARN(kModule,
                            "Proactive manifest fetch failed: depot=%u gid=%llu "
                            "(no API key, quota exhausted or provider error); "
                            "attempt %d/%d, next retry follows the backoff schedule.",
                            key.depotId, static_cast<unsigned long long>(key.gid),
                            attempts, kMaxProactiveRetries);
            }
        } catch (const std::exception& e) {
            AC_LOG_ERROR(kModule, "Proactive manifest worker failed: %s", e.what());
        } catch (...) {
            AC_LOG_ERROR(kModule, "Proactive manifest worker failed with unknown exception.");
        }
    }
}

// Queues a background fetch with backoff gating. Call from any thread WITHOUT
// holding g_proactiveMutex or g_state.manifestFetch.mutex (the async-lookup
// continuation calls this only after releasing the latter). Returns true when
// the key was actually queued.
bool EnqueueProactive(const LookupKey& key) {
    std::lock_guard<std::mutex> lock(g_proactiveMutex);
    auto it = g_proactiveStates.find(key);
    if (it == g_proactiveStates.end()) {
        if (g_proactiveStates.size() >= kMaxTrackedProactiveKeys) return false;
        it = g_proactiveStates.emplace(key, ProactiveState{}).first;
    }
    ProactiveState& state = it->second;
    if (state.attempts >= kMaxProactiveRetries) {
        AC_LOG_DEBUG_ONCE(kModule,
                          "Proactive retries exhausted for depot=%u gid=%llu this session.",
                          key.depotId, static_cast<unsigned long long>(key.gid));
        return false;
    }
    const auto now = std::chrono::steady_clock::now();
    if (now < state.nextEligible) return false;  // inside the backoff window
    state.nextEligible = now + std::chrono::seconds(kProactiveBackoffSec[state.attempts]);
    ++state.attempts;
    g_proactiveQueue.push_back(key);
    if (!g_proactiveWorkerStarted) {
        g_proactiveWorkerStarted = true;
        if (!workers::StartWorker("manifest_proactive", ProactiveWorkerLoop)) {
            g_proactiveWorkerStarted = false;   // riprova al prossimo enqueue
            AC_LOG_WARN(kModule, "Proactive manifest worker rejected (workers shut down).");
        }
    }
    g_proactiveCv.notify_one();
    return true;
}

}  // namespace

bool HasLocalManifest(std::uint64_t manifestGid, std::uint32_t depotId) {
    return FindLocalManifest(manifestGid, depotId).has_value();
}

void EnsureManifestAvailable(std::uint32_t appId, std::uint32_t depotId,
                             std::uint64_t manifestGid) {
    if (depotId == 0 || manifestGid == 0) return;

    const LookupKey key{manifestGid, appId, depotId};

    // Local first: FindLocalManifest also publishes a backup copy into
    // depotcache, so a manifest that exists anywhere on disk never reaches
    // the network. A local hit also clears this key's retry schedule.
    if (FindLocalManifest(manifestGid, depotId)) {
        std::lock_guard<std::mutex> lock(g_proactiveMutex);
        g_proactiveStates.erase(key);
        return;
    }

    // The dependency hook refires repeatedly while Steam downloads/retries;
    // EnqueueProactive gates resubmission with a per-key backoff schedule, so
    // a failed fetch is retried automatically (bounded per session) without
    // request storms.
    if (EnqueueProactive(key)) {
        AC_LOG_INFO(kModule, "Proactive manifest fetch queued: app=%u depot=%u gid=%llu.",
                    appId, depotId, static_cast<unsigned long long>(manifestGid));
    }
}

}  // namespace ac::manifestfetch
