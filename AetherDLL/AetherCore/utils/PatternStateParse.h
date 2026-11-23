#pragma once

#include <cstddef>
#include <cstdint>
#include <string>
#include <string_view>
#include <unordered_map>

// ---------------------------------------------------------------------------
// PatternStateParse — pure parser for the published generation index.
//
// The generator publishes `state/patterns.txt` on the release branch next to
// the tables it describes:
//
//     # Aether patterns schema versioning.
//
//     [steamclient]
//     <sha256> <generation>
//
// A table keeps its path forever (`<module>/<sha>.toml`), so a corrected table
// is republished in place. The index is how a consumer notices that what it
// reads at that path changed: the same decoded content keeps its generation,
// changed content gets the next one, and a table published for the first time
// starts at 1.
//
// This module touches neither g_state nor the network, so the "pattern_state"
// unit suite verifies it in isolation. PatternState owns the fetch and
// PatternCache owns the decision.
// ---------------------------------------------------------------------------

namespace ac::pattern {

// sha256 (lowercase) -> generation, for one module.
using ModuleGenerations = std::unordered_map<std::string, std::uint64_t>;
// module -> the tables it publishes.
using StateIndex = std::unordered_map<std::string, ModuleGenerations>;

struct StateParse {
    StateIndex modules;
    // Lines that did not contribute a new entry: malformed lines, and
    // duplicate entries (merged by keeping the larger generation). A published
    // file should never produce one; a hand-edited file may.
    std::size_t rejected = 0;
};

// Parses a whole index. Never throws and never rejects the document: an
// unreadable entry is counted in `rejected` and the rest stays usable.
StateParse ParseState(std::string_view body);

// Generation published for one table, 0 when the index carries no entry for
// it. Generations start at 1, so 0 means "unknown" and never a published value.
std::uint64_t Lookup(const StateIndex& index, std::string_view module, std::string_view sha);

} // namespace ac::pattern
