#pragma once

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <unordered_set>
#include <vector>

#include "scripting/LuaData.h"

namespace ac::hooks {

// Steam reallocates its depot table for us, but not unboundedly: a .lua file
// that lists a whole store's depots must not grow it past what one build pass
// can absorb. The cap is arbitrary but generous (the largest hand-authored
// .lua seen lists a few dozen depots).
inline constexpr std::size_t kMaxAppendedDepots = 512;

// Rows the dependency plan is missing: pinned depots with a real manifest that
// Steam did not list on its own. Pure so the decision is testable without
// Steam: sorted by depot id (stable plan), zero-gid pins dropped (a pin without
// a manifest targets nothing), capped at kMaxAppendedDepots.
inline std::vector<luadata::DepotPin> ComputeMissingPins(const std::vector<luadata::DepotPin>& pins,
                                                        const std::vector<steam::AppId>& presentIds) {
    const std::unordered_set<steam::AppId> present(presentIds.begin(), presentIds.end());
    std::vector<luadata::DepotPin> missing;
    missing.reserve(pins.size());
    for (const auto& pin : pins) {
        if (pin.depotId == 0 || pin.gid == 0 || present.count(pin.depotId))
            continue;
        missing.push_back(pin);
    }
    std::sort(missing.begin(), missing.end(),
              [](const luadata::DepotPin& a, const luadata::DepotPin& b) { return a.depotId < b.depotId; });
    if (missing.size() > kMaxAppendedDepots)
        missing.resize(kMaxAppendedDepots);
    return missing;
}

} // namespace ac::hooks
