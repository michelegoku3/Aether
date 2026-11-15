#pragma once

#include <cstdint>

// ============================================================================
// AchievementBits — which bits of a 5466 stat commit are new achievements.
//
// A stat commit carries two unrelated things under the same shape: an
// achievement bitfield (stat_id = the schema bucket, one bit per achievement)
// and a plain counter (a number). Only the schema tells them apart, because it
// lists the bits of an achievement bucket and the type of a counter.
//
// Bits below 8 are the ambiguous ones: a counter reaches them first, and some
// games put their first achievements there too (Paystreak's "Sold" is bucket 0,
// bit 4). A single threshold cannot serve both, so the caller passes what the
// schema said about the stat.
// ============================================================================
namespace ac::hooks::wire::bitfield {

// Bits set in `value` that `old` did not have and that belong to an
// achievement: every bit for a schema bucket, bits 8..31 otherwise.
inline std::uint32_t NewAchievementBits(std::uint32_t value, std::uint32_t old, bool schemaBucket) {
    constexpr std::uint32_t kCounterMask = 0xFFFFFF00u; // bits 8..31
    return (value & ~old) & (schemaBucket ? 0xFFFFFFFFu : kCounterMask);
}

} // namespace ac::hooks::wire::bitfield
