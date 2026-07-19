#pragma once

#include <cstddef>
#include <cstdint>
#include <functional>
#include <optional>
#include <span>
#include <string>
#include <vector>

#include "core/SteamTypes.h"

namespace ac::eticketfetch {

struct MintKey {
    steam::AppId appId = 0;
    std::string nonceHex;

    bool operator==(const MintKey&) const = default;
};

struct MintKeyHash {
    std::size_t operator()(const MintKey& key) const noexcept {
        return (static_cast<std::size_t>(key.appId) << 1) ^ std::hash<std::string>{}(key.nonceHex);
    }
};

struct TicketPair {
    std::vector<std::uint8_t> eticket;
    std::vector<std::uint8_t> ownership;
};

std::optional<TicketPair> Mint(steam::AppId appId, std::span<const std::uint8_t> nonce);
std::size_t CacheCount();

}  // namespace ac::eticketfetch
