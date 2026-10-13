#pragma once

#include <algorithm>
#include <cstdint>
#include <string>
#include <vector>

#include "core/SteamTypes.h"

// ============================================================================
// UserStatsSnapshot — JSON store of the achievement/stat state per
// (account, app): "aether-achievement-mirror-v1" format, one object per line
// (load = line scan, no full JSON parser). Owns the monotone MERGE RULES:
// content can only grow (0 = unknown date replaced by a real time; oldest
// time wins; stat = latest value).
// ============================================================================
namespace ac::backup::snapshot {

struct UnlockEntry {
    std::uint32_t id = 0;          // protocol achievement_id (eMsg 5466)
    std::uint32_t unlockTime = 0;  // unix time; 0 = unknown date (baseline)
};

struct StatEntry {
    std::uint32_t id = 0;     // protocol stat_id (for bitfields = bucket)
    std::uint32_t value = 0;  // last committed value
};

struct SnapshotData {
    std::vector<UnlockEntry> unlocks;
    std::vector<StatEntry> stats;
};

// Snapshot file path per (app, account). "" when backup unavailable.
std::string SnapshotPath(steam::AppId appId, std::uint32_t accountId);

// Loads a snapshot (missing file = empty, no error).
SnapshotData Load(const std::string& path);

// Atomic write (.tmp + move) of the whole snapshot.
void Save(const std::string& path, steam::AppId appId, std::uint32_t accountId,
          std::uint64_t steamId64, const SnapshotData& snap);

// --- Monotone merge rules (header-only: pure and testable without linking) ----
// Adds an unlock or refreshes its date (0 is replaced by a real time; between
// two real times the oldest wins).
inline void MergeUnlock(SnapshotData& snap, std::uint32_t achievementId, std::uint32_t unlockTime) {
    for (auto& e : snap.unlocks) {
        if (e.id != achievementId) continue;
        // Rule: the OLDEST time wins, but 0 (unknown baseline) is replaced by
        // any real time.
        if (e.unlockTime == 0) e.unlockTime = unlockTime;
        else if (unlockTime != 0 && unlockTime < e.unlockTime) e.unlockTime = unlockTime;
        return;
    }
    snap.unlocks.push_back(UnlockEntry{achievementId, unlockTime});
}

// Refreshes a stat's value (last committed value wins).
inline void MergeStat(SnapshotData& snap, std::uint32_t statId, std::uint32_t value) {
    for (auto& st : snap.stats) {
        if (st.id == statId) {
            st.value = value;   // last committed value wins
            return;
        }
    }
    snap.stats.push_back(StatEntry{statId, value});
}

inline bool HasUnlock(const SnapshotData& snap, std::uint32_t achievementId) {
    return std::any_of(snap.unlocks.begin(), snap.unlocks.end(),
                       [achievementId](const UnlockEntry& e) { return e.id == achievementId; });
}

inline bool HasStat(const SnapshotData& snap, std::uint32_t statId) {
    return std::any_of(snap.stats.begin(), snap.stats.end(),
                       [statId](const StatEntry& st) { return st.id == statId; });
}

// ascending id (deterministic output)
inline void SortAll(SnapshotData& snap) {
    std::sort(snap.unlocks.begin(), snap.unlocks.end(),
              [](const UnlockEntry& a, const UnlockEntry& b) { return a.id < b.id; });
    std::sort(snap.stats.begin(), snap.stats.end(),
              [](const StatEntry& a, const StatEntry& b) { return a.id < b.id; });
}

}  // namespace ac::backup::snapshot
