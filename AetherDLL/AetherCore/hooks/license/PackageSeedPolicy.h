#pragma once

#include <cstddef>
#include <cstdint>
#include <unordered_set>
#include <vector>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Package-seed policy: how one id set is folded into a package's live id vector.
//
// Package 0 is Steam's own object. Steam builds it before the scripts are read
// and keeps its storage for the life of the process, so the seed may only add
// what is missing: every id Steam already wrote stays where it is, in the order
// it chose, including duplicates the seed does not name. The transformation is
// pure - plain vectors in, plain vectors out - so the unit suite exercises the
// real thing before any pointer arithmetic touches Valve's storage.
//
// The same call runs over both of package 0's id vectors. They are separate
// because Steam answers two different questions from them: `appIdVec` is "is this
// app owned" and `depotIdVec` is "is this content licensed". Seeding only the
// first shows the card and plans nothing.
// ---------------------------------------------------------------------------
namespace ac::hooks::license::policy {

struct SeedOutcome {
    // The vector after the seed: Steam's ids first, then the new ones.
    std::vector<steam::AppId> ids;
    // Seeded ids that were already there once.
    std::size_t presentBefore = 0;
    // Extra copies of seeded ids that the compaction dropped.
    std::size_t duplicatesRemoved = 0;
    // Seeded ids that had to be added.
    std::size_t appended = 0;
};

// Zero ids and repeated seed ids collapse; an id already present is not added
// again; a seeded id present twice keeps its first copy. Ids outside the seed
// are never inspected beyond the membership test, so Steam's own duplicates
// survive exactly as they were.
inline SeedOutcome ApplySeed(const std::vector<steam::AppId>& current, const std::vector<steam::AppId>& seed) {
    std::vector<steam::AppId> wanted;
    wanted.reserve(seed.size());
    std::unordered_set<steam::AppId> want;
    want.reserve(seed.size());
    for (const steam::AppId id : seed) {
        if (id != 0 && want.insert(id).second)
            wanted.push_back(id);
    }

    SeedOutcome out;
    out.ids.reserve(current.size() + wanted.size());
    std::unordered_set<steam::AppId> seen;
    seen.reserve(wanted.size());
    for (const steam::AppId id : current) {
        if (want.count(id) == 0) {
            out.ids.push_back(id);
            continue;
        }
        if (!seen.insert(id).second) {
            ++out.duplicatesRemoved;
            continue;
        }
        ++out.presentBefore;
        out.ids.push_back(id);
    }
    for (const steam::AppId id : wanted) {
        if (seen.count(id) == 0)
            out.ids.push_back(id);
    }
    out.appended = wanted.size() - out.presentBefore;
    return out;
}

// The seed as it will be counted: zero and repeats removed, first-seen order
// kept.
inline std::vector<steam::AppId> NormalizeSeed(const std::vector<steam::AppId>& seed) {
    std::vector<steam::AppId> out;
    out.reserve(seed.size());
    std::unordered_set<steam::AppId> seen;
    seen.reserve(seed.size());
    for (const steam::AppId id : seed) {
        if (id != 0 && seen.insert(id).second)
            out.push_back(id);
    }
    return out;
}

// Outcome of pruning a package-0 id vector (preserving Steam's order).
struct PruneOutcome {
    std::vector<steam::AppId> ids;
    std::size_t removed = 0;
};

struct DualPruneOutcome {
    PruneOutcome apps;
    PruneOutcome depots;

    [[nodiscard]] bool Changed() const noexcept { return apps.removed != 0 || depots.removed != 0; }
};

inline std::unordered_set<steam::AppId> BuildNonZeroSet(const std::vector<steam::AppId>& ids) {
    std::unordered_set<steam::AppId> out;
    out.reserve(ids.size());
    for (const steam::AppId id : ids) {
        if (id != 0)
            out.insert(id);
    }
    return out;
}

// Removes every entry contained in `pruneSet` from `data[0..size)`, compacting
// surviving elements in-place while preserving their original order.
inline std::size_t PruneInPlace(steam::AppId* data, std::uint32_t& size,
                                const std::unordered_set<steam::AppId>& pruneSet) noexcept {
    if (!data || size == 0 || pruneSet.empty())
        return 0;
    std::size_t removed = 0;
    std::uint32_t write = 0;
    for (std::uint32_t read = 0; read < size; ++read) {
        if (pruneSet.count(data[read]) != 0) {
            ++removed;
            continue;
        }
        if (write != read)
            data[write] = data[read];
        ++write;
    }
    size = write;
    return removed;
}

// Pure mirror of PruneInPlace over an immutable vector for unit verification.
inline PruneOutcome ApplyPrune(const std::vector<steam::AppId>& current, const std::vector<steam::AppId>& pruneIds) {
    PruneOutcome out;
    out.ids = current;
    if (out.ids.empty() || pruneIds.empty())
        return out;
    const auto pruneSet = BuildNonZeroSet(pruneIds);
    std::uint32_t size = static_cast<std::uint32_t>(out.ids.size());
    out.removed = PruneInPlace(out.ids.data(), size, pruneSet);
    out.ids.resize(size);
    return out;
}

// Prunes matching root ids from both `appIdVec` and `depotIdVec` while leaving
// companion DLC/depot ids (`!= rootAppId`) untouched in both vectors.
inline DualPruneOutcome PrunePackageVectors(const std::vector<steam::AppId>& appIds,
                                            const std::vector<steam::AppId>& depotIds,
                                            const std::vector<steam::AppId>& pruneIds) {
    DualPruneOutcome out;
    out.apps = ApplyPrune(appIds, pruneIds);
    out.depots = ApplyPrune(depotIds, pruneIds);
    return out;
}

// Normalizes `seed` (dropping zero and duplicates) and strips `excludedIds`.
inline std::vector<steam::AppId> ExcludeIds(const std::vector<steam::AppId>& seed,
                                            const std::vector<steam::AppId>& excludedIds) {
    return ApplyPrune(NormalizeSeed(seed), excludedIds).ids;
}

// Outcome of a package-0 seed + license-refresh attempt. Seeding package 0's
// vectors can succeed before CUser is captured; the refresh itself stays
// pending until CUser exists and Steam is asked to re-read licenses.
enum class PublishStep {
    Unseeded,
    RefreshPending,
    Published,
};

constexpr PublishStep DecidePublishStep(bool package0Acquired, bool seedChanged, bool refreshed) noexcept {
    if (!package0Acquired)
        return PublishStep::Unseeded;
    if (seedChanged && !refreshed)
        return PublishStep::RefreshPending;
    return PublishStep::Published;
}

} // namespace ac::hooks::license::policy
