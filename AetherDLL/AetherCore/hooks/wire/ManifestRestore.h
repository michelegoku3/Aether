#pragma once

// ============================================================================
// ManifestRestore — backup/restore of Steam manifests into AetherData.
//
//   <AetherData>\backup\<app_id>\lua\*.manifest
//
// At Steam startup the module first copies into AetherData the manifests still
// present in Steam\depotcache, then restores the missing ones. DirWatch also
// watches Steam\steamapps\appmanifest_*.acf: after an ACF removal
// (uninstall) it re-triggers the app-targeted restore.
// ============================================================================

#include <cstddef>
#include <cstdint>
#include <filesystem>

namespace ac::hooks::ManifestRestore {

// Async manifest scan + backup/restore at startup, once per process.
// Best-effort and non-blocking for Steam initialization.
void RestoreMissingManifestsAtStartup();

// Synchronously restores the indicated app's manifests after the removal of
// appmanifest_<app_id>.acf. Called from the DirWatch thread after debounce.
void RestoreMissingManifestsForApp(std::uint32_t appId);

// Archives in AetherData a Hubcap-generated manifest
// (backup/<app_id>/lua/<depot>_<gid>.manifest) for every app whose Lua
// references it (active or commented pins). Called from the generation worker
// right after the depotcache install: the startup backup would only cover the
// file at the next Steam launch, and AetherDesk may not be running.
// Best-effort: follows the shared copy contract (never overwrite a non-empty
// destination, atomic verified copy) and never throws.
void BackupManifestAfterGeneration(std::uint32_t depotId, std::uint64_t gid);

struct RestoreStats {
    std::size_t scanned = 0;
    std::size_t restored = 0;
    std::size_t present = 0;
    std::size_t errors = 0;
};

// Test seam (also used by RestoreMissingManifestsForApp): copies manifests
// referenced by luaDir into depotcache without overwriting valid files;
// truncated-but-nonempty results count as errors, never as present hits.
RestoreStats RestoreLuaDirectory(const std::filesystem::path& luaDir, const std::filesystem::path& depotcache);

} // namespace ac::hooks::ManifestRestore
