#pragma once

#include <cstdint>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

// ---------------------------------------------------------------------------
// Hex <-> bytes conversion.
//
// Single source of truth for the whole project. Callers must treat Decode
// failure (nullopt) as malformed input — never invent padding.
// ---------------------------------------------------------------------------
namespace ac::hex {

// Decodes an even-length hex string into bytes. Returns nullopt on any invalid
// character or on odd length.
std::optional<std::vector<std::uint8_t>> Decode(std::string_view hex);

// Encodes bytes to uppercase hex (no 0x prefix). Empty input → empty string.
std::string Encode(std::span<const std::uint8_t> data);

}  // namespace ac::hex
