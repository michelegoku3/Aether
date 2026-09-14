#pragma once

#include <cstddef>
#include <cstdint>
#include <optional>

namespace ac::manifestfetch {

struct LookupKey {
    std::uint64_t gid = 0;
    std::uint32_t appId = 0;
    std::uint32_t depotId = 0;

    bool operator==(const LookupKey&) const = default;
};

struct LookupKeyHash {
    std::size_t operator()(const LookupKey& key) const noexcept {
        return static_cast<std::size_t>(key.gid) ^
               (static_cast<std::size_t>(key.gid >> 32) << 1) ^
               (static_cast<std::size_t>(key.appId) << 2) ^
               (static_cast<std::size_t>(key.depotId) << 3);
    }
};

// Returns true when the exact depot/GID manifest is already available in
// Steam's depotcache or the per-game AetherData backup.
bool HasLocalManifest(std::uint64_t manifestGid, std::uint32_t depotId);

void Submit(std::uint64_t jobId, std::uint64_t manifestGid,
            std::uint32_t appId, std::uint32_t depotId);

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

// Waits at most settings.manifestBridgeWaitMs (hard bound, default 2500 ms,
// 0 = pure passthrough) for the pending lookup of a wire job, so Steam's CM
// network thread never stalls on a slow generation. Returns the request code
// to inject (0 = satisfied from a local manifest) or nullopt when the
// original CM reply must pass through — the lookup keeps running in the
// background and Steam's retry gets the instant local hit once the manifest
// has been installed.
std::optional<std::uint64_t> Resolve(std::uint64_t jobId);
std::size_t PendingCount();
std::size_t CacheCount();

}  // namespace ac::manifestfetch
