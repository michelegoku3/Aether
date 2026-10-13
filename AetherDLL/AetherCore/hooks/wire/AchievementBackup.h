#pragma once

#include <cstdint>
#include <string>
#include <utility>
#include <vector>

#include "core/SteamTypes.h"

// ============================================================================
// AchievementBackup — loss-prevention persistence for achievement unlocks.
//
// Why it exists: the Steam server never confirms StoreUserStats2 for a game
// the account does not own, so the only copy of progress is the client's
// local cache (appcache\stats\UserGameStats_<account>_<appid>.bin,
// recognizable by PendingChanges > 0). Any client-side invalidation of that
// cache is a permanent loss. This module keeps an independent copy of it,
// with Steam's same per-(account, app) naming:
//
//   <AetherData>\backup\<appid>\achievements\            [via desk_path.cfg;
//                                               single destination: no
//                                               AetherDesk means no backup]
//       UserGameStats_<account>_<appid>.json   unlock snapshot (readable;
//                                               regenerates the .bin with
//                                               Tools\achievement_decoder.py)
//       UserGameStats_<account>_<appid>.bin    copy of the Steam cache
//       UserGameStatsSchema_<appid>.bin        copy of the game schema
//
// Contract (minimal facade, minimal coupling with AchievementModule):
//   * RecordUnlock() is ASYNC: no disk I/O on Steam's network thread — work is
//     queued to a dedicated worker (started on the first unlock). The snapshot
//     is rewritten atomically (.tmp + move) with merge (duplicate id -> oldest
//     time wins).
//   * FlushOnShutdown() blocks, drains the queue (including delayed jobs),
//     copies the .bins and awaits completion (join in the Workers registry).
//     Only on the explicit path outside the loader lock, NEVER in DllMain.
//     On normal termination protection comes from the periodic checkpoint +
//     the SessionEnded final copy.
//   * Everything is best-effort: an I/O error is only logged (WARN) and never
//     interferes with network traffic.
// ============================================================================

namespace ac::hooks::AchievementBackup {

// "1787052569" -> "18/08/2026 13:29:29" (local time). Shared with
// AchievementModule for log lines; here because it describes unlocks.
std::string FormatUnixTime(std::uint64_t unixTime);

// Enqueues one unlock's backup (see contract above). Thread-safe.
void RecordUnlock(steam::AppId appId, std::uint64_t steamId64, std::uint32_t achievementId, std::uint32_t unlockTime);

// "Session" backup: on an app's first stats use (818, 151 or 5466), copy the
// existing Steam cache .bins so accumulated progress is protected even when
// the session produces no new unlocks.
// Once per app per Steam process. Thread-safe.
void TouchSession(steam::AppId appId, std::uint64_t steamId64);

// EVENT-DRIVEN persistence (loss prevention even without shutdown):
//   * the worker runs a PERIODIC CHECKPOINT (5 min): forced re-copy of the
//     .bins of every touched (app, account) + playtime refresh;
//   * SessionEnded() schedules a DELAYED (~15 s) FINAL COPY when the game
//     exits, because Steam writes the stats cache after the empty frame.
// With these two events the maximum loss window drops from "whole session"
// to "5 minutes" even when Steam is closed/terminated normally.
void SessionEnded(steam::AppId appId, std::uint64_t steamId64);

// Startup snapshot (once per process): copies the .bins of ALL managed apps
// present in appcache\stats, right after Steam starts and before the client's
// login-reconcile can discard pending changes (the window where the 21/08
// loss happened). Async.
void BackupAllKnownStatsAtStartup();

// Records one commit's numeric stats (including achievement bitfields: many
// games — e.g. Spider-Man — commit achievements as stat_id=<bucket>,
// value=<bitfield>) into the JSON snapshot. Async.
void RecordStats(steam::AppId appId, std::uint64_t steamId64,
                 const std::vector<std::pair<std::uint32_t, std::uint32_t>>& stats);

// Drains the queue, copies the .bins one last time and stops the worker. Blocks
// until all pending work is complete (explicit shutdown only, never in DllMain).
void FlushOnShutdown();

} // namespace ac::hooks::AchievementBackup
