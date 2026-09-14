#include "pch.h"
#include "network/ManifestFetch.h"

#include <algorithm>
#include <chrono>
#include <cctype>
#include <charconv>
#include <condition_variable>
#include <deque>
#include <exception>
#include <filesystem>
#include <fstream>
#include <future>
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
constexpr std::size_t kMaxPendingJobs = 256;
constexpr std::size_t kMaxInflightLookups = 128;
constexpr std::size_t kMaxCacheEntries = 1024;

std::string ExpandTemplate(std::string_view tmpl, std::uint64_t gid,
                           std::uint32_t appId, std::uint32_t depotId) {
    std::string out;
    out.reserve(tmpl.size() + 32);
    for (std::size_t i = 0; i < tmpl.size();) {
        if (tmpl[i] != '{') {
            out.push_back(tmpl[i++]);
            continue;
        }
        const std::size_t end = tmpl.find('}', i + 1);
        if (end == std::string_view::npos) {
            out.push_back(tmpl[i++]);
            continue;
        }
        const std::string_view tag = tmpl.substr(i + 1, end - i - 1);
        if (tag == "gid") out += std::to_string(gid);
        else if (tag == "appid") out += std::to_string(appId);
        else if (tag == "depotid") out += std::to_string(depotId);
        else out.append(tmpl.substr(i, end - i + 1));
        i = end + 1;
    }
    return out;
}

bool EqualsIgnoreCase(std::string_view a, std::string_view b) {
    if (a.size() != b.size()) return false;
    for (std::size_t i = 0; i < a.size(); ++i) {
        const unsigned char ac = static_cast<unsigned char>(a[i]);
        const unsigned char bc = static_cast<unsigned char>(b[i]);
        if (std::tolower(ac) != std::tolower(bc)) return false;
    }
    return true;
}

std::string_view ExtractHost(std::string_view url) {
    std::size_t begin = 0;
    const std::size_t scheme = url.find("://");
    if (scheme != std::string_view::npos) begin = scheme + 3;
    std::size_t end = url.find_first_of("/?#", begin);
    std::string_view host = end == std::string_view::npos
        ? url.substr(begin)
        : url.substr(begin, end - begin);
    const std::size_t at = host.rfind('@');
    if (at != std::string_view::npos) host.remove_prefix(at + 1);
    const std::size_t port = host.find(':');
    if (port != std::string_view::npos) host = host.substr(0, port);
    return host;
}

bool IsTrustedHost(std::string_view host) {
    if (host.empty()) return false;
    for (const auto& trusted : g_state.settings.manifestFetchTrustedHosts) {
        if (EqualsIgnoreCase(host, trusted)) return true;
    }
    return false;
}

bool IsSupportedProviderUrl(std::string_view url, std::string_view host) {
    const bool https = url.size() >= 8 && EqualsIgnoreCase(url.substr(0, 8), "https://");
    const bool http = url.size() >= 7 && EqualsIgnoreCase(url.substr(0, 7), "http://");
    if ((!https && !http) || host.empty()) return false;
    // Credentials are not needed for configured manifest providers and make
    // host parsing/redirect auditing unnecessarily ambiguous.
    const std::size_t schemeEnd = url.find("://");
    const std::size_t authorityEnd = url.find_first_of("/?#", schemeEnd == std::string_view::npos ? 0 : schemeEnd + 3);
    const std::size_t at = url.find('@', schemeEnd == std::string_view::npos ? 0 : schemeEnd + 3);
    return at == std::string_view::npos ||
           (authorityEnd != std::string_view::npos && at > authorityEnd);
}

bool UsesProviderCompatAgent(std::string_view url) {
    return EqualsIgnoreCase(ExtractHost(url), "manifest.opensteamtool.com");
}

bool ParseDigitsOnly(std::string_view body, std::uint64_t* out) {
    if (!out) return false;
    std::size_t b = 0;
    std::size_t e = body.size();
    while (b < e && (body[b] == ' ' || body[b] == '\r' || body[b] == '\n' || body[b] == '\t')) ++b;
    while (e > b && (body[e - 1] == ' ' || body[e - 1] == '\r' || body[e - 1] == '\n' || body[e - 1] == '\t')) --e;
    if (b == e) return false;
    std::uint64_t value = 0;
    auto [_, ec] = std::from_chars(body.data() + b, body.data() + e, value);
    if (ec != std::errc{} || value == 0) return false;
    *out = value;
    return true;
}

bool ParseJsonDigitField(std::string_view body, std::uint64_t* out) {
    if (!out) return false;
    static constexpr std::string_view kKeys[] = {
        "\"manifest_request_code\"", "\"content\"", "\"code\"",
    };
    for (auto key : kKeys) {
        const std::size_t k = body.find(key);
        if (k == std::string_view::npos) continue;
        std::size_t pos = body.find(':', k + key.size());
        if (pos == std::string_view::npos) continue;
        while (pos + 1 < body.size() && std::isspace(static_cast<unsigned char>(body[pos + 1]))) ++pos;
        ++pos;
        if (pos >= body.size()) continue;

        if (body[pos] == '"') {
            const std::size_t end = body.find('"', pos + 1);
            if (end != std::string_view::npos &&
                ParseDigitsOnly(body.substr(pos + 1, end - pos - 1), out)) {
                return true;
            }
            continue;
        }

        const std::size_t end = body.find_first_not_of("0123456789", pos);
        const std::string_view digits = body.substr(
            pos, end == std::string_view::npos ? body.size() - pos : end - pos);
        if (ParseDigitsOnly(digits, out)) return true;
    }
    return false;
}

// A manifest already present on disk is authoritative for this bridge. Steam
// still asks ContentServerDirectory for a request code when a depot is marked
// as owned by Aether, but a cached manifest does not need a network-issued
// code. Returning an engaged optional containing zero lets ManifestBridge
// convert the failed service response into an OK response without contacting a
// patched/unauthenticated provider.
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
            if (!ec && usable(destination)) return destination;
            AC_LOG_WARN(kModule,
                        "Local backup manifest found but could not be published to %s (%s).",
                        destination.string().c_str(), ec.message().c_str());
        }
    }
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

std::optional<std::uint64_t> RunLookup(std::uint64_t gid, std::uint32_t appId,
                                       std::uint32_t depotId) {
    // Production path: obtain the exact manifest directly from authenticated
    // Hubcap at the same request-code synchronization point. AetherDesk is not
    // required to be running; its DPAPI-protected credential file is read by
    // the DLL under the current Windows user.
    if (InstallHubcapManifest(gid, depotId)) return std::uint64_t{0};

    if (g_state.settings.manifestFetchUrls.empty()) {
        AC_LOG_DEBUG(kModule, "gid=%llu skipped, no providers configured.",
                     static_cast<unsigned long long>(gid));
        return std::nullopt;
    }

    for (std::size_t i = 0; i < g_state.settings.manifestFetchUrls.size(); ++i) {
        const std::string& tmpl = g_state.settings.manifestFetchUrls[i];
        if (tmpl.empty()) continue;

        const std::string url = ExpandTemplate(tmpl, gid, appId, depotId);
        const std::string_view host = ExtractHost(url);
        if (!IsSupportedProviderUrl(url, host) || !IsTrustedHost(host)) {
            AC_LOG_WARN(kModule, "gid=%llu provider %zu skipped, URL/host not trusted.",
                        static_cast<unsigned long long>(gid), i + 1);
            continue;
        }

        AC_LOG_INFO(kModule, "gid=%llu provider %zu/%zu GET %s",
                    static_cast<unsigned long long>(gid), i + 1,
                    g_state.settings.manifestFetchUrls.size(), url.c_str());

        http::Response resp;
        for (int attempt = 0; attempt < 2; ++attempt) {
            resp = UsesProviderCompatAgent(url)
                ? http::GetUnchecked(url, g_state.settings.manifestFetchTimeoutSec, L"OpenSteamTool/1.0")
                : http::GetUnchecked(url, g_state.settings.manifestFetchTimeoutSec);
            if (!resp.networkError && resp.status == 429 && attempt == 0) {
                AC_LOG_WARN(kModule, "gid=%llu provider %zu HTTP=429, retrying once.",
                            static_cast<unsigned long long>(gid), i + 1);
                std::this_thread::sleep_for(std::chrono::milliseconds(750));
                continue;
            }
            break;
        }

        if (resp.networkError) {
            AC_LOG_WARN(kModule, "gid=%llu provider %zu network error, trying next.",
                        static_cast<unsigned long long>(gid), i + 1);
            continue;
        }
        if (resp.status != 200) {
            AC_LOG_WARN(kModule, "gid=%llu provider %zu HTTP=%d, trying next.",
                        static_cast<unsigned long long>(gid), i + 1, resp.status);
            continue;
        }

        std::uint64_t code = 0;
        if (ParseDigitsOnly(resp.body, &code) || ParseJsonDigitField(resp.body, &code)) {
            AC_LOG_INFO(kModule, "gid=%llu resolved code=%llu via provider %zu.",
                        static_cast<unsigned long long>(gid),
                        static_cast<unsigned long long>(code), i + 1);
            return code;
        }

        AC_LOG_WARN(kModule, "gid=%llu provider %zu body unparseable, trying next.",
                    static_cast<unsigned long long>(gid), i + 1);
    }

    AC_LOG_WARN(kModule, "gid=%llu all providers exhausted.",
                static_cast<unsigned long long>(gid));
    return std::nullopt;
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

void ProactiveWorkerLoop() {
    for (;;) {
        LookupKey key{};
        {
            std::unique_lock<std::mutex> lock(g_proactiveMutex);
            g_proactiveCv.wait(lock, [] { return !g_proactiveQueue.empty(); });
            key = g_proactiveQueue.front();
            g_proactiveQueue.pop_front();
        }
        try {
            // Another path (wire-bridge lookup, ManifestRestore, AetherDesk)
            // may have installed the manifest meanwhile: re-check before
            // spending shared quota.
            if (HasLocalManifest(key.gid, key.depotId)) {
                std::lock_guard<std::mutex> lock(g_proactiveMutex);
                g_proactiveStates.erase(key);
                continue;
            }
            {
                // A wire-bridge lookup may still own this key; its failure
                // continuation requeues with backoff, so skip instead of
                // double-generating.
                std::lock_guard<std::mutex> lock(g_state.manifestFetch.mutex);
                if (g_state.manifestFetch.inflight.count(key) > 0) continue;
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
        std::thread(ProactiveWorkerLoop).detach();
    }
    g_proactiveCv.notify_one();
    return true;
}

}  // namespace

bool HasLocalManifest(std::uint64_t manifestGid, std::uint32_t depotId) {
    return FindLocalManifest(manifestGid, depotId).has_value();
}

void Submit(std::uint64_t jobId, std::uint64_t manifestGid,
            std::uint32_t appId, std::uint32_t depotId) {
    if (jobId == 0 || manifestGid == 0 || appId == 0 || depotId == 0) {
        AC_LOG_WARN(kModule, "Rejected invalid manifest lookup identifiers.");
        return;
    }

    const LookupKey key{manifestGid, appId, depotId};
    std::lock_guard<std::mutex> lock(g_state.manifestFetch.mutex);
    if (g_state.manifestFetch.pending.count(jobId)) {
        AC_LOG_DEBUG(kModule, "Duplicate submit for job=%llu ignored.",
                     static_cast<unsigned long long>(jobId));
        return;
    }
    if (g_state.manifestFetch.pending.size() >= kMaxPendingJobs) {
        AC_LOG_WARN(kModule, "Manifest pending limit reached; job=%llu rejected.",
                    static_cast<unsigned long long>(jobId));
        return;
    }

    // Prefer the exact depot/GID file restored by ManifestRestore (or already
    // present in either Steam depotcache location). A zero request code is the
    // local-cache sentinel; it is never sent to an HTTP provider and therefore
    // does not depend on unauthenticated Steam request-code access.
    if (const auto local = FindLocalManifest(manifestGid, depotId)) {
        std::promise<std::optional<std::uint64_t>> ready;
        ready.set_value(std::optional<std::uint64_t>{0});
        g_state.manifestFetch.pending.emplace(jobId, ready.get_future().share());
        g_state.manifestFetch.cache[key] = 0;
        AC_LOG_INFO(kModule,
                    "job=%llu gid=%llu local manifest hit: %s; skipping providers.",
                    static_cast<unsigned long long>(jobId),
                    static_cast<unsigned long long>(manifestGid),
                    local->string().c_str());
        return;
    }

    if (auto cached = g_state.manifestFetch.cache.find(key);
        cached != g_state.manifestFetch.cache.end()) {
        std::promise<std::optional<std::uint64_t>> ready;
        ready.set_value(cached->second);
        g_state.manifestFetch.pending.emplace(jobId, ready.get_future().share());
        AC_LOG_INFO(kModule, "job=%llu gid=%llu served from cache.",
                    static_cast<unsigned long long>(jobId),
                    static_cast<unsigned long long>(manifestGid));
        return;
    }

    if (auto inflight = g_state.manifestFetch.inflight.find(key);
        inflight != g_state.manifestFetch.inflight.end()) {
        g_state.manifestFetch.pending.emplace(jobId, inflight->second);
        AC_LOG_INFO(kModule, "job=%llu gid=%llu joined in-flight lookup.",
                    static_cast<unsigned long long>(jobId),
                    static_cast<unsigned long long>(manifestGid));
        return;
    }

    if (g_state.manifestFetch.inflight.size() >= kMaxInflightLookups) {
        AC_LOG_WARN(kModule, "Manifest in-flight limit reached; job=%llu rejected.",
                    static_cast<unsigned long long>(jobId));
        return;
    }

    // Manual promise + detached thread instead of std::async(launch::async).
    // The async shared state JOINS its thread from its own destructor when the
    // last future handle dies, and both places where that can happen here are
    // unacceptable: on the CM network thread (the Resolve timeout path erases
    // 'pending', so the destructor would block the wire until the worker
    // finishes) or on the worker thread itself (the in-flight erase below can
    // be the last handle -> self-join -> std::system_error EDEADLK ->
    // terminate; reproduced on libstdc++). A promise-based shared state never
    // joins; it is simply released when the last handle goes away.
    auto resultPromise = std::make_shared<std::promise<std::optional<std::uint64_t>>>();
    std::shared_future<std::optional<std::uint64_t>> fut = resultPromise->get_future().share();
    auto startPromise = std::make_shared<std::promise<void>>();
    const std::shared_future<void> startGate = startPromise->get_future().share();
    try {
        std::thread([key, startGate, resultPromise]() {
            startGate.wait();
            std::optional<std::uint64_t> result;
            try {
                result = RunLookup(key.gid, key.appId, key.depotId);
            } catch (const std::exception& e) {
                AC_LOG_ERROR(kModule, "Manifest lookup worker failed: %s", e.what());
            } catch (...) {
                AC_LOG_ERROR(kModule, "Manifest lookup worker failed with unknown exception.");
            }

            {
                std::lock_guard<std::mutex> lock(g_state.manifestFetch.mutex);
                if (result) {
                    if (g_state.manifestFetch.cache.size() >= kMaxCacheEntries) {
                        g_state.manifestFetch.cache.erase(g_state.manifestFetch.cache.begin());
                    }
                    g_state.manifestFetch.cache[key] = *result;
                }
                g_state.manifestFetch.inflight.erase(key);
            }
            if (!result) {
                // Continuation: keep retrying in the background with backoff
                // even if Steam never resubmits this job. Queued only after
                // the in-flight entry is gone so the worker does not
                // self-skip.
                EnqueueProactive(key);
            }
            // Satisfy waiters only after all bookkeeping is committed.
            try {
                resultPromise->set_value(result);
            } catch (...) {
                AC_LOG_ERROR(kModule, "Manifest lookup result could not be published.");
            }
        }).detach();
    } catch (const std::exception& e) {
        AC_LOG_ERROR(kModule, "Manifest lookup scheduling failed: %s", e.what());
        return;
    } catch (...) {
        AC_LOG_ERROR(kModule, "Manifest lookup scheduling failed with unknown exception.");
        return;
    }

    g_state.manifestFetch.inflight.emplace(key, fut);
    g_state.manifestFetch.pending.emplace(jobId, fut);
    startPromise->set_value();
    AC_LOG_INFO(kModule, "job=%llu gid=%llu lookup started.",
                static_cast<unsigned long long>(jobId),
                static_cast<unsigned long long>(manifestGid));
}

std::optional<std::uint64_t> Resolve(std::uint64_t jobId) {
    std::shared_future<std::optional<std::uint64_t>> fut;
    {
        std::lock_guard<std::mutex> lock(g_state.manifestFetch.mutex);
        auto it = g_state.manifestFetch.pending.find(jobId);
        if (it == g_state.manifestFetch.pending.end()) return std::nullopt;
        fut = it->second;
        g_state.manifestFetch.pending.erase(it);
    }

    // Hard, short bound — this runs on Steam's CM network thread. The typical
    // Hubcap generation (~1 s) fits inside the default 2500 ms window; anything
    // slower passes the original CM reply through (the job fails fast instead
    // of stalling wire traffic and heartbeats) while this lookup keeps running
    // in the background. Once the manifest lands in depotcache, Steam's own
    // retry resubmits and gets the instant local hit (code 0).
    const int waitMs = g_state.settings.manifestBridgeWaitMs >= 0
        ? std::min(g_state.settings.manifestBridgeWaitMs, 10000)
        : 2500;
    if (fut.wait_for(std::chrono::milliseconds(waitMs)) != std::future_status::ready) {
        AC_LOG_WARN(kModule,
                    "job=%llu not ready within the %dms bridge window; passing the "
                    "original CM reply through (background fetch continues, Steam's "
                    "retry picks up the installed manifest).",
                    static_cast<unsigned long long>(jobId), waitMs);
        diag::Record("manifest_timeout", std::to_string(jobId));
        return std::nullopt;
    }
    try {
        return fut.get();
    } catch (const std::exception& e) {
        AC_LOG_ERROR(kModule, "job=%llu result retrieval failed: %s.",
                     static_cast<unsigned long long>(jobId), e.what());
        return std::nullopt;
    } catch (...) {
        AC_LOG_ERROR(kModule, "job=%llu result retrieval failed with unknown exception.",
                     static_cast<unsigned long long>(jobId));
        return std::nullopt;
    }
}

std::size_t PendingCount() {
    std::lock_guard<std::mutex> lock(g_state.manifestFetch.mutex);
    return g_state.manifestFetch.pending.size();
}

std::size_t CacheCount() {
    std::lock_guard<std::mutex> lock(g_state.manifestFetch.mutex);
    return g_state.manifestFetch.cache.size();
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
