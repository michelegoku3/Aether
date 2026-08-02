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

// Asynchronous, non-blocking encrypted-ticket minting (A1).
//
// The HTTP POST to the configured eticket backend runs on a dedicated worker
// thread, never on the caller's thread (which may be Steam's IPC thread).
//   * cache hit        -> returns immediately (no network);
//   * already in flight-> returns immediately (single-flight, one POST per key);
//   * otherwise        -> queues the key and returns immediately.
// The worker persists minted tickets to the registry + cache, so a later
// RequestEncryptedAppTicket finds them. This API is best-effort: it never
// blocks and never throws.
void MintAsync(steam::AppId appId, std::span<const std::uint8_t> nonce);

// Number of keys currently queued/in-flight (diagnostics).
std::size_t InflightCount();

// Number of cached ticket pairs (diagnostics).
std::size_t CacheCount();

// Stops and joins the worker thread. Safe to call when never started.
void Shutdown();

}  // namespace ac::eticketfetch
