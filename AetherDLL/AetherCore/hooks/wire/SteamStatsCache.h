#pragma once

#include <cstdint>
#include <string>
#include <unordered_set>

#include "core/SteamTypes.h"

// ============================================================================
// SteamStatsCache — access to Steam's stats cache files (appcache\stats):
// .bin copies with a MONOTONE GUARD (never overwrite a richer backup with a
// just-wiped cache) and rate-limit; achievement counting of binary .bins;
// schema interpretation (achievement buckets) to translate bitfield stats
// into achievements.
//
// Internal state (per-app schema cache, per-(app,account) rate-limit): owned
// by the facade's worker thread — no internal concurrency needed.
// ============================================================================
namespace ac::backup::statscache {

// Copies UserGameStats_<account>_<appid>.bin + UserGameStatsSchema_<appid>.bin
// into the backup. `force` skips the rate-limit (unlocks and final copies).
void BackupStatsBins(steam::AppId appId, std::uint32_t accountId, bool force = false);

// Achievements present in a UserGameStats .bin (sum of the "data" bitfield
// bits); -1 when unreadable. Used by the monotone guard.
int CountAchievementsInBinFile(const std::string& path);

// The game's achievement bucket set read from the schema
// (UserGameStatsSchema_<appid>.bin; per-process cache). Buckets with an empty
// "bits" are NOT achievement buckets.
const std::unordered_set<std::uint32_t>& SchemaBucketsFor(steam::AppId appId);

} // namespace ac::backup::statscache
