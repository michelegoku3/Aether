#include "pch.h"
#include "utils/PatternSource.h"

#include <array>

namespace ac::downloader {

namespace {

// The generation index lives at a fixed name in its folder; per-build tables
// use "<sha>.toml". Both share the same URL shape.
constexpr const char* kStateFileName = "patterns.txt";

// Builds the raw.githubusercontent.com URL for one file in a location.
std::string BuildRawUrl(const Source& src, const Loc& loc, const std::string& fileName) {
    std::string url = "https://raw.githubusercontent.com/";
    url += src.owner;
    url += "/";
    url += src.repo;
    url += "/";
    url += loc.branch;
    url += "/";
    url += loc.folder;
    url += "/";
    url += fileName;
    return url;
}

// Builds the jsDelivr CDN mirror of the same file (same owner/repo/branch).
std::string BuildCdnUrl(const Source& src, const Loc& loc, const std::string& fileName) {
    std::string url = "https://cdn.jsdelivr.net/gh/";
    url += src.owner;
    url += "/";
    url += src.repo;
    url += "@";
    url += loc.branch;
    url += "/";
    url += loc.folder;
    url += "/";
    url += fileName;
    return url;
}

std::vector<MirrorUrl> UrlsIn(const Source& src, const Loc& loc, const std::string& fileName) {
    return {MirrorUrl{"raw", BuildRawUrl(src, loc, fileName)}, MirrorUrl{"cdn", BuildCdnUrl(src, loc, fileName)}};
}

} // namespace

std::vector<MirrorUrl> Source::UrlsFor(Kind kind, const std::string& sha) const {
    const Loc* loc = LocFor(kind);
    if (!loc)
        return {}; // this source does not carry that kind
    return UrlsIn(*this, *loc, sha + ".toml");
}

std::vector<MirrorUrl> Source::UrlsForState() const {
    if (state.branch == nullptr || state.folder == nullptr)
        return {}; // this source does not carry the index
    // Raw only. The CDN serves branch refs from an edge cache that outlives a
    // republish, and the index is the one file republished in place, so a CDN
    // copy can only hide the change; one endpoint keeps the boot wait at one
    // timeout too.
    return {MirrorUrl{"raw", BuildRawUrl(*this, state, kStateFileName)}};
}

const std::vector<Source>& DefaultSources() {
    // Priority is defined by array order (index 0 = tried first). Keep the
    // comment above each entry in sync with the layout it publishes.
    // MigoReleases is the only built-in source: the primary author publishes
    // a table for every supported build there, so no fallback upstream is
    // needed.
    static const std::array<Source, 1> kSources = {{
        {
            /* id        */ "migo",
            /* display   */ "michelegoku3 / MigoReleases",
            /* owner     */ "michelegoku3",
            /* repo      */ "MigoReleases",
            /* steamClient    */ {"pattern", "steamclient"},
            /* steamUi        */ {"pattern", "steamui"},
            /* steamClientIpc */ {"pattern", "steamclientipc"},
            /* state          */ {"pattern", "state"},
        },
    }};
    static const std::vector<Source> kOrdered(kSources.begin(), kSources.end());
    return kOrdered;
}

} // namespace ac::downloader
