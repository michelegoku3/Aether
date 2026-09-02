#pragma once

// ---------------------------------------------------------------------------
// PatternSource.h — ordered, data-driven model of remote pattern sources.
//
// AetherCore no longer hard-codes a single upstream repo for per-build TOML
// files. Instead, each "source" is described by a small, immutable record:
//
//   * the GitHub owner/repo that hosts the files, and
//   * a per-resource layout telling the downloader where inside that repo a
//     given logical file lives (branch + folder).
//
// Sources are evaluated strictly in the order they appear in the registry
// (kBuiltInSources): position = priority. The first source that can serve the
// requested build wins; later sources are only consulted as fallbacks.
//
// This file is intentionally free of I/O. Its single responsibility is to model
// sources and translate a (kind, sha) request into candidate mirror URLs.
// Fetching and persistence live in PatternDownloader.
//
// To add/remove/reorder an upstream, edit kBuiltInSources. Each entry is
// self-contained, so a new repo (or a repo whose IPC files sit on a different
// branch, as OpenSteamTool's do) requires no changes to the downloader logic.
// ---------------------------------------------------------------------------

#include <string>
#include <string_view>
#include <vector>

namespace ac::downloader {

// The logical per-build files AetherCore can ask a source for.
enum class Kind {
    SteamClient,     // byte-pattern offsets inside steamclient64.dll
    SteamUi,         // byte-pattern offsets inside steamui.dll
    SteamClientIpc,  // IPC method spec for steamclient64.dll
};

// Canonical, human/disk-friendly name for a Kind. Also used as the {subdir}
// placeholder when expanding a user-supplied mirror template (KoriaPolis-style
// layout, where IPC lives under the "steamclientipc" folder).
constexpr const char* KindName(Kind kind) {
    switch (kind) {
        case Kind::SteamClient:    return "steamclient";
        case Kind::SteamUi:        return "steamui";
        case Kind::SteamClientIpc: return "steamclientipc";
    }
    return "unknown";
}

// Inverse of KindName: parses a module/dir name ("steamclient", "steamui",
// "steamclientipc") back into a Kind. Unknown strings default to
// Kind::SteamClient so callers that treat "steamclient" as the generic module
// keep working.
inline Kind KindFromName(std::string_view name) {
    if (name == "steamui")        return Kind::SteamUi;
    if (name == "steamclientipc") return Kind::SteamClientIpc;
    return Kind::SteamClient;  // "steamclient" (and anything unknown)
}

// One candidate transport endpoint for a file (e.g. "raw" or "cdn").
struct MirrorUrl {
    const char* label;  // stable, shown in logs/status, e.g. "raw" / "cdn"
    std::string url;
};

// Location of a given Kind inside a GitHub repo (branch + folder).
struct Loc {
    const char* branch;  // e.g. "pattern" or "ipc"
    const char* folder;  // e.g. "steamclient"
};

// Immutable description of a remote pattern source.
struct Source {
    const char* id;       // stable machine key used in logs/status (e.g. "koriapolis")
    const char* display;  // human-readable label for logs (e.g. "KoriaPolis / Steam-Auto-PT")
    const char* owner;    // GitHub owner
    const char* repo;     // GitHub repo
    Loc steamClient;      // layout for Kind::SteamClient
    Loc steamUi;          // layout for Kind::SteamUi
    Loc steamClientIpc;   // layout for Kind::SteamClientIpc

    // Returns the layout for a Kind, or nullptr if this source does not carry
    // that kind of file.
    const Loc* LocFor(Kind kind) const {
        switch (kind) {
            case Kind::SteamClient:    return &steamClient;
            case Kind::SteamUi:        return &steamUi;
            case Kind::SteamClientIpc: return &steamClientIpc;
        }
        return nullptr;
    }

    // Builds the ordered candidate URLs (GitHub raw, then jsDelivr CDN) that
    // could serve `sha` for `kind`. Empty when this source has no layout for
    // that kind.
    std::vector<MirrorUrl> UrlsFor(Kind kind, const std::string& sha) const;
};

// Default ordered source chain. Ordering == priority: index 0 is tried first.
//
// Current policy: KoriaPolis first (it publishes the historical reference
// layout), OpenSteamTool second — so if KoriaPolis has not yet published the
// pattern for the current Steam build, the downloader automatically falls back
// to OpenSteamTool.
const std::vector<Source>& DefaultSources();

}  // namespace ac::downloader
