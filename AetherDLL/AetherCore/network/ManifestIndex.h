#pragma once

#include <filesystem>
#include <optional>
#include <string>

// ---------------------------------------------------------------------------
// ManifestIndex — inverted lookup over the AetherData backup tree.
//
// Maps manifest filename (<depot>_<gid>.manifest) to the backup lua dir
// holding it. Built by ONE full walk per session, then O(1) per lookup;
// backup writes upsert instead of waiting out negative memos. Returned
// paths are re-verified by the caller before use (files may be pruned by
// the user or Desk between index and read).
// ---------------------------------------------------------------------------
namespace ac::manifestfetch::index {

// Finds filename under backupRoot (walked once per session). Returns the
// full backup path when the index knows a usable file, nullopt otherwise.
// Stale entries (known but no longer usable) are dropped.
std::optional<std::filesystem::path> FindInBackup(const std::filesystem::path& backupRoot, const std::string& filename);

// Upserts after a backup write lands on disk. No-op when the index was
// never built (the next lookup builds it and merges this entry).
void NoteFileAdded(const std::string& filename, const std::filesystem::path& backupLuaDir);

} // namespace ac::manifestfetch::index
