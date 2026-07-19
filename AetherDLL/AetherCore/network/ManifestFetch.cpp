#include "pch.h"
#include "network/ManifestFetch.h"

#include <algorithm>
#include <chrono>
#include <cctype>
#include <charconv>
#include <exception>
#include <future>
#include <memory>
#include <optional>
#include <string>
#include <string_view>
#include <thread>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "network/RuntimeHttp.h"

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

std::optional<std::uint64_t> RunLookup(std::uint64_t gid, std::uint32_t appId,
                                       std::uint32_t depotId) {
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

}  // namespace

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

    std::shared_future<std::optional<std::uint64_t>> fut;
    auto startPromise = std::make_shared<std::promise<void>>();
    const std::shared_future<void> startGate = startPromise->get_future().share();
    try {
        fut = std::async(std::launch::async, [key, startGate]() -> std::optional<std::uint64_t> {
        startGate.wait();
        std::optional<std::uint64_t> result;
        try {
            result = RunLookup(key.gid, key.appId, key.depotId);
        } catch (const std::exception& e) {
            AC_LOG_ERROR(kModule, "Manifest lookup worker failed: %s", e.what());
        } catch (...) {
            AC_LOG_ERROR(kModule, "Manifest lookup worker failed with unknown exception.");
        }

        std::lock_guard<std::mutex> lock(g_state.manifestFetch.mutex);
        if (result) {
            if (g_state.manifestFetch.cache.size() >= kMaxCacheEntries) {
                g_state.manifestFetch.cache.erase(g_state.manifestFetch.cache.begin());
            }
            g_state.manifestFetch.cache[key] = *result;
        }
        g_state.manifestFetch.inflight.erase(key);
        return result;
        }).share();
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

    const int timeout = g_state.settings.manifestFetchTimeoutSec > 0
        ? g_state.settings.manifestFetchTimeoutSec : 12;
    if (fut.wait_for(std::chrono::seconds(timeout)) != std::future_status::ready) {
        AC_LOG_WARN(kModule, "job=%llu timed out after %ds.",
                    static_cast<unsigned long long>(jobId), timeout);
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

}  // namespace ac::manifestfetch
