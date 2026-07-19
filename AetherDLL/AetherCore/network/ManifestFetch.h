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

void Submit(std::uint64_t jobId, std::uint64_t manifestGid,
            std::uint32_t appId, std::uint32_t depotId);
std::optional<std::uint64_t> Resolve(std::uint64_t jobId);
std::size_t PendingCount();
std::size_t CacheCount();

}  // namespace ac::manifestfetch
