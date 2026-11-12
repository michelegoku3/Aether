#pragma once

#include <cstddef>
#include <cstdint>
#include <unordered_map>

namespace ac::achieve {

// The game's stats schema (appcache\stats\UserGameStatsSchema_<appid>.bin)
// names the stat that drives each achievement's counter:
//
//   "<appid>" {
//       "stats" {
//           <bucket> {                           <- the achievement bucket
//               "bits" {
//                   <bit> {
//                       "name"     "<api name>"
//                       "progress" { "min_val" 0 "max_val" 40
//                                    "value" { "operation" "statvalue"
//                                              "operand1"  "STAT_7" } }
//                   }
//               }
//           }
//           <stat id> { "name" "STAT_7" "data" 0 }   <- the counter itself
//       }
//   }
//
// The reply to Player.GetUserAchievements#1 carries that counter in
// `progress_float`; without it the page draws an empty bar. The value is the
// raw stat value, not a fraction.
//
// Only `operation == "statvalue"` is read: every progress block in the
// client's schema corpus uses it, and an operation driven by another
// achievement has no value in the stats file. A block whose `operand1` names no
// stat in the same schema is dropped rather than guessed.
struct StatsSchema {
    // internal_key = bucket * 256 + bit  ->  stat id holding the current value
    std::unordered_map<std::uint32_t, std::uint32_t> progressStat;
};

// Parses a KV1 stats schema. Rows collected before a malformed entry stay
// usable; returns false only when the stream is malformed.
bool ParseStatsSchema(const std::uint8_t* data, std::size_t size, StatsSchema& out);

} // namespace ac::achieve
