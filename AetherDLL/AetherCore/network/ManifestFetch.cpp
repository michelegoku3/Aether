#include "pch.h"
#include "network/ManifestFetch.h"

#include <algorithm>
#include <chrono>
#include <cctype>
#include <charconv>
#include <future>
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
    if (ec != std::errc{}) return false;
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
        const std::size_t q1 = body.find('"', k + key.size());
        if (q1 == std::string_view::npos) continue;
        const std::size_t q2 = body.find('"', q1 + 1);
        if (q2 == std::string_view::npos) continue;
        if (ParseDigitsOnly(body.substr(q1 + 1, q2 - q1 - 1), out)) return true;
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
        if (!IsTrustedHost(host)) {
            AC_LOG_WARN(kModule, "gid=%llu provider %zu skipped, host '%.*s' not trusted.",
                        static_cast<unsigned long long>(gid), i + 1,
                        static_cast<int>(host.size()), host.data());
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
    const LookupKey key{manifestGid, appId, depotId};

    std::lock_guard<std::mutex> lock(g_state.manifestFetch.mutex);
    if (g_state.manifestFetch.pending.count(jobId)) {
        AC_LOG_DEBUG(kModule, "Duplicate submit for job=%llu ignored.",
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

    auto fut = std::async(std::launch::async, [key]() -> std::optional<std::uint64_t> {
        std::optional<std::uint64_t> result = RunLookup(key.gid, key.appId, key.depotId);
        std::lock_guard<std::mutex> lock(g_state.manifestFetch.mutex);
        if (result) g_state.manifestFetch.cache[key] = *result;
        g_state.manifestFetch.inflight.erase(key);
        return result;
    }).share();

    g_state.manifestFetch.inflight.emplace(key, fut);
    g_state.manifestFetch.pending.emplace(jobId, fut);
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
    return fut.get();
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
