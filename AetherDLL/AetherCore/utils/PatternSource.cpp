#include "pch.h"
#include "utils/PatternSource.h"

#include <array>

namespace ac::downloader {

namespace {

// Builds the raw.githubusercontent.com URL for one location.
std::string BuildRawUrl(const Source& src, const Loc& loc, const std::string& sha) {
    std::string url = "https://raw.githubusercontent.com/";
    url += src.owner;
    url += "/";
    url += src.repo;
    url += "/";
    url += loc.branch;
    url += "/";
    url += loc.folder;
    url += "/";
    url += sha;
    url += ".toml";
    return url;
}

// Builds the jsDelivr CDN mirror of the same file (same owner/repo/branch).
std::string BuildCdnUrl(const Source& src, const Loc& loc, const std::string& sha) {
    std::string url = "https://cdn.jsdelivr.net/gh/";
    url += src.owner;
    url += "/";
    url += src.repo;
    url += "@";
    url += loc.branch;
    url += "/";
    url += loc.folder;
    url += "/";
    url += sha;
    url += ".toml";
    return url;
}

}  // namespace

std::vector<MirrorUrl> Source::UrlsFor(Kind kind, const std::string& sha) const {
    std::vector<MirrorUrl> out;
    const Loc* loc = LocFor(kind);
    if (!loc) return out;  // this source does not carry that kind

    out.push_back(MirrorUrl{"raw", BuildRawUrl(*this, *loc, sha)});
    out.push_back(MirrorUrl{"cdn", BuildCdnUrl(*this, *loc, sha)});
    return out;
}

const std::vector<Source>& DefaultSources() {
    // Priority is defined by array order (index 0 = tried first). Keep the
    // comment above each entry in sync with the layout it publishes.
    static const std::array<Source, 2> kSources = {{
        {
            /* id        */ "koriapolis",
            /* display   */ "KoriaPolis / Steam-Auto-PT",
            /* owner     */ "KoriaPolis",
            /* repo      */ "Steam-Auto-PT",
            /* steamClient    */ {"pattern", "steamclient"},
            /* steamUi        */ {"pattern", "steamui"},
            /* steamClientIpc */ {"pattern", "steamclientipc"},
        },
        {
            /* id        */ "opensteamtool",
            /* display   */ "OpenSteam001 / steam-monitor",
            /* owner     */ "OpenSteam001",
            /* repo      */ "steam-monitor",
            /* steamClient    */ {"pattern", "steamclient"},
            /* steamUi        */ {"pattern", "steamui"},
            /* steamClientIpc */ {"ipc", "steamclient"},
        },
    }};
    static const std::vector<Source> kOrdered(kSources.begin(), kSources.end());
    return kOrdered;
}

}  // namespace ac::downloader
