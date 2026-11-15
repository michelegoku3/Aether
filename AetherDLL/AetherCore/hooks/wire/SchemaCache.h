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
// Only validated schemas are written (at least one achievement bucket), the same
// bytes are copied into the AetherData backup so a wiped cache stays
// recoverable, and a cache entry Steam wrote itself is authoritative: it is
// never replaced by anything but the client.
//
// The one exception is Aether's own transcription (StoreTranscribed): a schema
// built from a page capture is still a stand-in, so a later capture may replace
// it when the game changes. Ownership is recorded in a marker next to the backup
// copy (TranscribedSchema) rather than guessed from the bytes: the backup holds
// Steam's downloads too, so identical bytes prove nothing.
// ---------------------------------------------------------------------------
namespace ac::hooks::wire::schemacache {

// What Aether wrote into the cache: enough to recognise the file again and to
// tell whether a newer capture should replace it.
struct TranscribedSchema {
    bool present = false;
    std::int32_t version = 0; // the reply's schema_version
    std::uint32_t count = 0;  // achievements the capture listed
    std::uint64_t hash = 0;   // FNV-1a of the bytes written
};

// Marker file contents (small text, readable in a support bundle).
std::string EncodeTranscribed(const TranscribedSchema& marker);

// False for a missing, truncated or foreign file: a marker that does not parse
// means "not ours", never a guess.
bool ParseTranscribed(const std::string& text, TranscribedSchema& out);

// True when `cacheBytes` is exactly what the marker was written for.
bool IsTranscribedFile(const TranscribedSchema& marker, const std::string& cacheBytes);

enum class SchemaWrite {
    Skip, // leave the cache alone
    Write // the capture is newer than the file Aether put there
};

// Decision table for a capture: a missing cache is always written; a file that
// is not ours (Steam's, another donor's) never is; ours is rewritten only when
// the capture carries a newer schema, so an unchanged page open costs nothing.
SchemaWrite DecideWrite(const TranscribedSchema& marker, const std::string& cacheBytes, std::int32_t version,
                        std::uint32_t count);

// True when `schema` is a KV1 blob with at least one achievement bucket.
bool IsAchievementSchema(const std::string& schema);

// Writes the schema into `statsDir` (Steam's appcache\stats) and, when
// `backupDir` is not empty, into the per-app achievements backup as well.
// Returns true when a usable schema file is in place afterwards.
bool Store(steam::AppId appId, const std::string& schema, const std::string& statsDir, const std::string& backupDir);

// Publishes a schema Aether transcribed from a page capture (AchievementUi).
// Writes cache, backup and marker together, and replaces an earlier
// transcription when `version`/`count` moved on. A file Steam wrote is left
// untouched. True when usable bytes are in the cache afterwards; `applied`
// (optional) says whether this call wrote or found the file already current.
bool StoreTranscribed(steam::AppId appId, const std::string& schema, const std::string& statsDir,
                      const std::string& backupDir, std::int32_t version, std::uint32_t count,
                      SchemaWrite* applied = nullptr);

// Puts Aether's own copy of the schema back into the client cache when the
// cache has none, marker included, so the file stays recognisable as Aether's.
// The client builds a game's achievement list at login from that file, so a
// session that starts before the library page was opened has to be handed it
// back. True when a usable schema is in the cache afterwards.
bool Restore(steam::AppId appId, const std::string& statsDir, const std::string& backupDir);

// App id from a backup directory name ("<AetherData>\backup\<appid>"); 0 when
// the name is not one, so the startup restore walks only real app folders.
steam::AppId AppIdFromBackupDir(const std::string& name);

} // namespace ac::hooks::wire::schemacache
