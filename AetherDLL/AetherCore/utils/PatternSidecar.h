#pragma once

#include <cstdint>
#include <string>

// ---------------------------------------------------------------------------
// PatternSidecar — the provenance/version record stored next to a cached table.
//
// Every cache TOML (`<sha>.toml`) is paired with `<sha>.toml.src`:
//
//     migo:raw
//     version=3
//
// Line 1 is the source label that produced the file (the user mirror id, or
// "<source>:<mirror>"), which the provenance probe compares against the
// highest-priority source. Line 2 is the generation of the published table the
// cache holds, so a launch can tell without network whether the upstream table
// moved.
//
// A one-line sidecar (the label only) reads as generation 0 — a cache that no
// index has confirmed yet — so the next launch with a reachable index adopts
// the published generation once.
//
// Pure file I/O: no g_state, no network, so the "pattern_sidecar" unit suite
// verifies it in isolation.
// ---------------------------------------------------------------------------

namespace ac::pattern::sidecar {

struct Record {
    std::string source;        // empty when unknown
    std::uint64_t version = 0; // 0 when unknown: no version line, or one that does not parse
};

// Reads a sidecar. A missing or unreadable file yields the empty record.
Record Read(const std::string& path);

// Replaces the sidecar atomically (temp file + rename). False when the file
// could not be written; the caller keeps serving the cache either way.
bool Write(const std::string& path, const std::string& source, std::uint64_t version);

} // namespace ac::pattern::sidecar
