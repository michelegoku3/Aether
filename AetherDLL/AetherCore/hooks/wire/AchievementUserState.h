#pragma once

#include <cstddef>
#include <cstdint>
#include <unordered_map>

namespace ac::achieve {

// Local state read from Steam's UserGameStats_<account>_<appid>.bin.
//
// `unlocked` is keyed by internal_key = bucket * 256 + bit, the identity the
// client's achievement page uses. The value is the unlock time in unix seconds,
// 0 when the bit is set without a recorded date. A bit counts as unlocked when
// either the bucket's `data` mask, its `pendingbits` mask or an
// `AchievementTimes` entry carries it: for an app the account does not own the
// server never acks the bit, so it only ever appears in `pendingbits`, and the
// date can outlive the mask.
//
// `values` is keyed by stat id (the bucket) and holds the current value of the
// stat, which is what the client shows in the progress bar of an achievement
// whose counter is that stat (AchievementStatsSchema.h names the stat).
//
// The reader cannot tell an achievement bitfield from a stat: the schema reply
// decides, because only its `internal_key`s are ever looked up here
// (AchievementUiPolicy.h).
struct UserState {
    std::unordered_map<std::uint32_t, std::uint32_t> unlocked;
    std::unordered_map<std::uint32_t, float> values;

    bool empty() const { return unlocked.empty(); }
};

// Parses a KV1 stats file. Returns false on a malformed stream; rows collected
// before the error stay usable (partial file, partial state).
bool ParseUserState(const std::uint8_t* data, std::size_t size, UserState& out);

// Adds to `into` the unlocks and stat values only `from` carries.
//
// The client's cache (appcache\stats) is not durable: Steam rewrites it for an
// app the account does not own, and what it writes can be empty. Aether's own
// copy of the same file is what survives, so the page reads both and merges
// them. A key in both keeps the earlier unlock date (the first observation),
// and a stat keeps the value of the file being read.
void MergeUserState(UserState& into, const UserState& from);

// True when `snapshot` holds an unlock `live` does not.
//
// The game page's achievements block is built from the client's own stats file,
// not from the reply Aether rewrites, so a page whose file Steam emptied cannot
// be repaired by answering alone: the file has to be put back. Counting rows
// would miss the case this exists for - a file that lost one key and gained a
// stale one - so the question is asked per key.
bool HasUnlockMissingFrom(const UserState& snapshot, const UserState& live);

} // namespace ac::achieve
