#pragma once

#include <cstddef>
#include <cstdint>
#include <string_view>

// ---------------------------------------------------------------------------
// Steam content manifest identity validation (P17 testability).
//
// Extracted verbatim from ManifestFetch.cpp so the byte-level checks can be
// unit-tested with synthetic blobs. A Steam depot manifest starts with two
// fixed-size sections:
//   [0x71F617D0][u32 length][payload]   "ManifestInfo" header
//   [0x1F4812BE][u32 length][metadata]  protobuf-ish depot/GID descriptor
// ManifestIdentityMatches confirms the blob carries the depot id + global id
// we asked for before trusting any of its content.
// ---------------------------------------------------------------------------
namespace ac::manifestidentity {

bool ReadU32Le(std::string_view bytes, std::size_t& offset, std::uint32_t& out);
bool ReadVarint(std::string_view bytes, std::size_t& offset, std::uint64_t& out);

// Returns false on any structural problem (truncation, wrong magic, malformed
// protobuf framing) — the blob must then be treated as unusable.
bool ManifestIdentityMatches(std::string_view bytes, std::uint32_t expectedDepot,
                             std::uint64_t expectedGid);

}  // namespace ac::manifestidentity
