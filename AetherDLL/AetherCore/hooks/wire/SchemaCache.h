#pragma once

#include <string>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// SchemaCache — publishing a game's achievement schema into Steam's own cache.
//
// The Steam client stores the schema a donor response carried in
// <steam>\appcache\stats\UserGameStatsSchema_<appid>.bin (byte-identical: file
// size == the response's `schema` field). A donor found by the probe chain can
// therefore be made effective without waiting for the client to ask again: once
// the file is there, the client answers games from it like a normal download.
//
// Only validated schemas are written (at least one achievement bucket), an
// existing file is never replaced (a real Steam copy always wins) and the same
// bytes are copied into the AetherData backup so a wiped cache stays
// recoverable.
// ---------------------------------------------------------------------------
namespace ac::hooks::wire::schemacache {

// True when `schema` is a KV1 blob with at least one achievement bucket.
bool IsAchievementSchema(const std::string& schema);

// Writes the schema into `statsDir` (Steam's appcache\stats) and, when
// `backupDir` is not empty, into the per-app achievements backup as well.
// Returns true when a usable schema file is in place afterwards.
bool Store(steam::AppId appId, const std::string& schema, const std::string& statsDir, const std::string& backupDir);

} // namespace ac::hooks::wire::schemacache
