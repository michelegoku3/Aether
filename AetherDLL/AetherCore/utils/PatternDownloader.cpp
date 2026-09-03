#include "pch.h"
#include "utils/PatternDownloader.h"

#include <filesystem>
#include <fstream>
#include <string>
#include <string_view>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "network/RuntimeHttp.h"

namespace ac::downloader {
namespace {

constexpr const char* kModule = "PatternDownloader";

// Result of a single HTTP attempt. `definitiveMissing` is set when the server
// authoritatively reports 404 (as opposed to a transport error): the file does
// not exist at that endpoint, so any mirror of the SAME source is not worth
// consulting — CDNs also tend to cache 404s, so waiting for one would only add
// latency without changing the outcome.
struct FetchResult {
    bool ok = false;
    bool definitiveMissing = false;
    std::string body;
};

// Performs a single GET through RuntimeHttp's shared URL parser/timeouts.
// timeoutSec caps this attempt (short values for boot-time provenance probes).
FetchResult Fetch(const std::string& url, int timeoutSec) {
    FetchResult result;
    http::Response resp = http::GetUnchecked(url, timeoutSec);
    if (resp.networkError) {
        AC_LOG_WARN(kModule, "GET failed (network) for %s.", url.c_str());
        return result;  // transient: a mirror over another transport may work
    }
    if (resp.status == 404) {
        AC_LOG_INFO(kModule, "GET 404 for %s.", url.c_str());
        result.definitiveMissing = true;
        return result;
    }
    if (resp.status != 200) {
        AC_LOG_WARN(kModule, "GET failed for %s (status=%d).", url.c_str(), resp.status);
        return result;
    }
    if (resp.body.size() > constants::kMaxPatternResponseBytes) {
        AC_LOG_WARN(kModule, "Response exceeds size cap; aborting (%s).", url.c_str());
        return result;
    }
    result.ok = true;
    result.body = std::move(resp.body);
    return result;
}

// Creates every missing directory in `path` so the atomic write below can never
// fail just because a per-kind subfolder (e.g. steamclientipc) does not exist.
void EnsureParentDir(const std::string& path) {
    std::error_code ec;
    std::filesystem::create_directories(std::filesystem::path(path).parent_path(), ec);
}

// Writes via temp file + atomic rename so a partial write never corrupts cache.
bool SaveAtomic(const std::string& path, const std::string& content) {
    EnsureParentDir(path);
    const std::string tmp = path + ".tmp";
    {
        std::ofstream out(tmp, std::ios::binary | std::ios::trunc);
        if (!out.is_open()) return false;
        out.write(content.data(), static_cast<std::streamsize>(content.size()));
    }
    if (!MoveFileExA(tmp.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING)) {
        DeleteFileA(tmp.c_str());
        return false;
    }
    return true;
}

// Expands {subdir}/{sha} in a user-supplied mirror template.
std::string ApplyTemplate(std::string tmpl, std::string_view subdir, const std::string& sha) {
    auto replace = [&](const std::string& token, const std::string& value) {
        for (std::size_t p = tmpl.find(token); p != std::string::npos; p = tmpl.find(token, p)) {
            tmpl.replace(p, token.size(), value);
            p += value.size();
        }
    };
    replace("{subdir}", std::string(subdir));
    replace("{sha}", sha);
    return tmpl;
}

// One endpoint to try: a fully-built URL plus the stable label reported back on
// success (the user mirror id, or "<source>:<mirror>").
struct Candidate {
    std::string label;
    std::string url;
};

// Candidates at the same priority level are alternative transports serving the
// SAME data (raw vs CDN). The first success wins; an authoritative 404 from any
// endpoint ends the level immediately, because the source genuinely does not
// carry the file. Levels themselves are sources in insertion-priority order:
// level 0 must fail before level 1 is consulted.
struct Level {
    std::vector<Candidate> endpoints;
};

// Builds the strictly priority-ordered resolution plan: configured mirror first,
// then the built-in source registry (see PatternSource::DefaultSources).
std::vector<Level> BuildPlan(Kind kind, const std::string& sha) {
    std::vector<Level> plan;

    // Level 0: user-supplied mirror has the highest priority when configured.
    if (!g_state.settings.patternMirror.empty()) {
        Level level;
        level.endpoints.push_back(Candidate{
            "mirror", ApplyTemplate(g_state.settings.patternMirror, KindName(kind), sha)});
        plan.push_back(std::move(level));
    }

    // Following levels: built-in sources in registry order.
    for (const Source& src : DefaultSources()) {
        const auto mirrors = src.UrlsFor(kind, sha);
        if (mirrors.empty()) continue;  // this source does not carry that kind
        Level level;
        for (const MirrorUrl& m : mirrors) {
            level.endpoints.push_back(
                Candidate{std::string(src.id) + ":" + m.label, m.url});
        }
        plan.push_back(std::move(level));
    }
    return plan;
}

// Tries one source level. Endpoints are contacted in order (raw, then CDN):
//   * 200            -> the file is served;
//   * transport error -> the next endpoint (CDN) is tried, as it is a different
//                        network path to the identical file;
//   * 404            -> the source does not have the file: stop immediately
//                        (waiting on the CDN mirror only burns time).
// Returns the winning label (empty = level failed) and fills `outBody`.
std::string TryLevel(const Level& level, std::string& outBody, int timeoutSec) {
    for (const Candidate& ep : level.endpoints) {
        FetchResult res = Fetch(ep.url, timeoutSec);
        if (res.ok) {
            // The successful save is logged once by the caller (Download).
            outBody = std::move(res.body);
            return ep.label;
        }
        if (res.definitiveMissing) {
            AC_LOG_INFO(kModule, "Endpoint '%s' has no pattern yet (404); source skipped.",
                        ep.label.c_str());
            return {};
        }
        // transport error: try the next transport (CDN) of this source
        AC_LOG_INFO(kModule, "Endpoint '%s' unreachable; trying next transport of this source.",
                    ep.label.c_str());
    }
    return {};
}

}  // namespace

bool Download(Kind kind, const std::string& sha, const std::string& outPath,
              std::string* outSource, int timeoutSec) {
    AC_LOG_INFO(kModule, "Resolving '%s/%s'.", KindName(kind), sha.c_str());

    const std::vector<Level> plan = BuildPlan(kind, sha);
    if (plan.empty()) {
        AC_LOG_WARN(kModule, "No sources configured for '%s/%s'.", KindName(kind), sha.c_str());
        return false;
    }

    for (const Level& level : plan) {
        std::string body;
        const std::string label = TryLevel(level, body, timeoutSec);
        if (label.empty()) {
            AC_LOG_INFO(kModule, "Source level could not serve '%s/%s'; trying next source.",
                        KindName(kind), sha.c_str());
            continue;
        }
        if (!SaveAtomic(outPath, body)) {
            AC_LOG_WARN(kModule, "Failed to write %s; trying next source.", outPath.c_str());
            continue;
        }
        if (outSource) *outSource = label;
        AC_LOG_INFO(kModule, "Saved pattern from '%s': %s", label.c_str(), outPath.c_str());
        return true;
    }

    AC_LOG_WARN(kModule, "All sources failed for '%s/%s'.", KindName(kind), sha.c_str());
    return false;
}

bool Download(std::string_view kindName, const std::string& sha, const std::string& outPath,
              std::string* outSource, int timeoutSec) {
    return Download(KindFromName(kindName), sha, outPath, outSource, timeoutSec);
}

}  // namespace ac::downloader
