#pragma once

#include <cstddef>
#include <cstdint>
#include <unordered_map>

namespace ac::achieve {

// Unlocked achievements read from Steam's UserGameStats_<account>_<appid>.bin.
//
// Key = internal_key = bucket * 256 + bit, the identity the client's achievement
// page uses (measured on the beta client). The
// value is the unlock time in unix seconds, 0 when the bit is set without a
// recorded date.
//
// Buckets that carry a bitfield (`data`, and/or `pendingbits` for a bit the
// server has not confirmed yet) contribute one key per set bit. The reader
// cannot tell an achievement bitfield from a stat that happens to have a `data`
// int; the schema reply is what decides, because only its `internal_key`s are
// ever looked up here (AchievementUiPolicy.h). A bit is published when either
// mask holds it: for an app the account does not own nothing is confirmed, so
// the unlock only ever appears in `pendingbits`.
struct UserState {
    std::unordered_map<std::uint32_t, std::uint32_t> unlocked;

    bool empty() const { return unlocked.empty(); }
};

// Parses a KV1 stats file. Returns false on a malformed stream; rows collected
// before the error stay usable (partial file, partial state).
bool ParseUserState(const std::uint8_t* data, std::size_t size, UserState& out);

} // namespace ac::achieve
