#pragma once

#include <string>
#include <string_view>

#include "utils/PatternSource.h"

// ---------------------------------------------------------------------------
// PatternDownloader — fetches a per-build pattern TOML over HTTPS and stores it
// atomically in the local cache.
//
// The actual candidates to try come from the ordered source registry defined
// in PatternSource (see ac::downloader::DefaultSources). This module is only
// responsible for the fetch-orchestration and the atomic write:
//
//   1. if a user-supplied mirror is configured it is tried first (highest
//      priority);
//   2. otherwise the built-in sources are tried strictly in registry order —
//      insertion order IS priority, and a source must fail before the next one
//      is consulted;
//   3. within one source, the raw endpoint is tried first; the CDN mirror is
//      only contacted on a transport error (different network path to the
//      identical file). An authoritative HTTP 404 means the source simply does
//      not carry that build, so the next source is consulted immediately
//      without waiting on the CDN (which would also 404, possibly from cache).
//
// Callers (PatternEngine, IpcSpec) resolve their independent files
// concurrently, so on a fresh Steam build — when every file misses the cache —
// the network latency is paid in parallel rather than serially.
//
// Each successful/falling-back step is logged so the user can see exactly which
// upstream served the file (or that none had it yet).
// ---------------------------------------------------------------------------
namespace ac::downloader {

// kind is one of the logical pattern kinds (see PatternSource.h); sha is the
// DLL's SHA-256; outPath is where the body is written on success. Returns true
// if any source/mirror succeeded. outSource, when non-null, receives a stable
// label describing what served the file (the user mirror id, or "<source>:<mirror>").
bool Download(Kind kind, const std::string& sha, const std::string& outPath,
              std::string* outSource = nullptr);

// Convenience overload: accepts the raw module/dir name ("steamclient",
// "steamui", "steamclientipc") and forwards to the Kind-based entry point.
// Accepts both std::string and string literals.
bool Download(std::string_view kindName, const std::string& sha, const std::string& outPath,
              std::string* outSource = nullptr);

}  // namespace ac::downloader
