#include "pch.h"
#include "utils/PatternDownloader.h"

#include <fstream>
#include <string>
#include <utility>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "network/RuntimeHttp.h"

namespace ac::downloader {
namespace {

constexpr const char* kModule = "PatternDownloader";

// Performs a single GET through RuntimeHttp's shared URL parser/timeouts.
// Returns the body on HTTP 200, otherwise empty.
std::string Fetch(const std::string& url) {
    http::Response resp = http::GetUnchecked(url, 12);
    if (resp.networkError || resp.status != 200) {
        AC_LOG_WARN(kModule, "GET failed for %s (status=%d, network=%d).", url.c_str(),
                    resp.status, resp.networkError ? 1 : 0);
        return {};
    }
    if (resp.body.size() > constants::kMaxPatternResponseBytes) {
        AC_LOG_WARN(kModule, "Response exceeds size cap; aborting.");
        return {};
    }
    return resp.body;
}


// Writes via temp file + atomic rename so a partial write never corrupts cache.
bool SaveAtomic(const std::string& path, const std::string& content) {
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

// Substitutes {subdir} and {sha} in a user-supplied mirror template.
std::string ApplyTemplate(std::string tmpl, const std::string& subdir, const std::string& sha) {
    auto replace = [&](const std::string& token, const std::string& value) {
        for (std::size_t p = tmpl.find(token); p != std::string::npos; p = tmpl.find(token, p)) {
            tmpl.replace(p, token.size(), value);
            p += value.size();
        }
    };
    replace("{subdir}", subdir);
    replace("{sha}", sha);
    return tmpl;
}

}  // namespace

bool Download(const std::string& subdir, const std::string& sha, const std::string& outPath,
              std::string* outSource) {
    AC_LOG_INFO(kModule, "Resolving pattern for %s/%s", subdir.c_str(), sha.c_str());

    std::vector<std::pair<std::string, std::string>> mirrors;
    // A configured mirror takes priority when present.
    if (!g_state.settings.patternMirror.empty()) {
        mirrors.emplace_back("mirror", ApplyTemplate(g_state.settings.patternMirror, subdir, sha));
    }
    // Built-in chain: GitHub raw, then the jsDelivr CDN mirror of the same repo.
    mirrors.emplace_back("github", "https://raw.githubusercontent.com/KoriaPolis/Steam-Auto-PT/pattern/" +
                         subdir + "/" + sha + ".toml");
    mirrors.emplace_back("cdn", "https://cdn.jsdelivr.net/gh/KoriaPolis/Steam-Auto-PT@pattern/" +
                         subdir + "/" + sha + ".toml");

    for (const auto& [label, url] : mirrors) {
        std::string body = Fetch(url);
        if (!body.empty() && SaveAtomic(outPath, body)) {
            if (outSource) *outSource = label;
            AC_LOG_INFO(kModule, "Saved pattern from %s: %s", label.c_str(), outPath.c_str());
            return true;
        }
    }

    AC_LOG_WARN(kModule, "All mirrors failed for %s/%s.", subdir.c_str(), sha.c_str());
    return false;
}

}  // namespace ac::downloader
