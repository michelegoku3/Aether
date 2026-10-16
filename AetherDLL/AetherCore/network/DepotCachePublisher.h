#pragma once

#include <cstdint>
#include <filesystem>
#include <optional>
#include <string>

// ---------------------------------------------------------------------------
// DepotCachePublisher — backup -> Steam depotcache publication.
//
// Copies one manifest file into Steam's depotcache (mkdir + overwrite copy
// + usability verification). Stateless and lock-free: callers serialize
// policy decisions; concurrent publishes of the same file are idempotent
// (same bytes under the depot_gid identity).
// ---------------------------------------------------------------------------
namespace ac::manifestfetch::publisher {

// True when path is a usable manifest (regular file, non-empty).
bool IsUsableManifestFile(const std::filesystem::path& path);

// Publishes source into <steamRoot>/depotcache/<filename>. Returns the
// destination on success, nullopt (logged) otherwise.
std::optional<std::filesystem::path> Publish(const std::filesystem::path& source,
                                             const std::filesystem::path& steamRoot, const std::string& filename);

} // namespace ac::manifestfetch::publisher
