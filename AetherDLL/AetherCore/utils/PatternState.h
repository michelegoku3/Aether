#pragma once

#include <cstdint>
#include <string>

// ---------------------------------------------------------------------------
// PatternState — the generation index, resolved once per process.
//
// The generator publishes `state/patterns.txt` next to the tables (see
// PatternStateParse for the format). One GET answers "did any table I care
// about change since I cached it?", which is cheaper and more precise than the
// provenance probe: a corrected table keeps its path, so only the index can
// announce that the bytes at that path moved.
//
// The fetch happens once, on the first module that asks, through the built-in
// sources only. A configured user mirror is the user's own source of truth and
// may not carry the index at all, so the index is not consulted then and the
// provenance policy alone decides.
//
// This header stays dependency-free (no g_state, no network) so the pure
// decision below is unit tested; PatternState.cpp owns the fetch.
// ---------------------------------------------------------------------------

namespace ac::pattern {

// Generation published for `sha` in `module`; 0 when the index is missing,
// unreachable, has no entry for that pair, or a custom mirror is configured.
// The first call performs the one fetch the process makes; the resolved source
// is published to `g_state.patternIndexSource` for status.json.
std::uint64_t RemoteVersion(const std::string& module, const std::string& sha);

// What a launch does with a cached table.
struct RefreshPlan {
    bool needed = false;
    // A newer generation is the author's explicit statement that the table
    // changed: adopt it even when it is smaller than the cache. The provenance
    // probe keeps the never-shrink rule, because a fallback source serving
    // fewer entries is a downgrade, not a correction.
    bool versioned = false;
};

constexpr RefreshPlan PlanRefresh(bool cacheUsable, std::uint64_t remote, std::uint64_t applied, bool provenanceBest) {
    if (!cacheUsable)
        return {}; // nothing cached: the caller downloads a table as usual
    if (remote > applied)
        return {true, true};
    return {!provenanceBest, false};
}

// A downloaded candidate replaces the cache unless the index claims a newer
// generation, the cache already records a generation, and the bytes are exactly
// what the cache holds. The index carries no digest of the table, so identical
// bytes after a bump mean an endpoint re-served a stale copy, not a change:
// keeping the recorded generation makes the next launch retry. A cache that
// records no generation adopts what it already holds and records the published
// one, so it converges on the first launch instead of downloading the same
// bytes forever.
constexpr bool AdoptDownload(bool versioned, std::uint64_t applied, bool sameBytes) {
    return !(versioned && applied > 0 && sameBytes);
}

} // namespace ac::pattern
