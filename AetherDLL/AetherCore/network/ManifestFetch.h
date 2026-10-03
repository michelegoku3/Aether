#pragma once

#include <cstdint>

// Manifest acquisition (post legacy wire-bridge removal).
//
// The ONLY production paths are:
//   * local:   FindLocalManifest/HasLocalManifest — Steam depotcache or the
//              per-game AetherData backup (indexed, negative entries cached);
//   * Hubcap:  authenticated generation serialized on a worker, triggered by
//              EnsureManifestAvailable from the dependency-build hook.
// The old GetManifestRequestCode wire-bridge (Submit/Resolve + provider URL
// templates) was unreachable on current Steam builds and has been removed.
namespace ac::manifestfetch {

// Returns true when the exact depot/GID manifest is already available in
// Steam's depotcache or the per-game AetherData backup.
bool HasLocalManifest(std::uint64_t manifestGid, std::uint32_t depotId);

// Proactive acquisition: make sure the exact depot/GID manifest Steam just
// selected exists in Steam's depotcache. Local sources first (a backup copy
// is published into depotcache); anything really missing is generated through
// the authenticated Hubcap pipeline on a serialized worker thread (shared
// quota file, atomic install). Never blocks the caller: Steam's own retry
// picks the file up once it has landed in depotcache. Failed fetches are
// retried in the background with a bounded backoff schedule (max 6 attempts
// per depot/GID per session) instead of the old one-shot-per-session.
void EnsureManifestAvailable(std::uint32_t appId, std::uint32_t depotId,
                             std::uint64_t manifestGid);

}  // namespace ac::manifestfetch
