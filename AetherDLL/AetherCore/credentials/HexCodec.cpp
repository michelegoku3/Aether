#include "pch.h"
#include "credentials/HexCodec.h"
#include "core/Logger.h"

namespace ac::hex {
namespace {

constexpr const char* kModule = "HexCodec";

// Returns the value 0-15 for a hex digit, or -1 if the character is not hex.
int NibbleValue(char c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
}

}  // namespace

std::optional<std::vector<std::uint8_t>> Decode(std::string_view hex) {
    // Odd-length input cannot represent whole bytes; reject it outright rather
    // than silently padding (which would produce a wrong key/ticket).
    if (hex.size() % 2 != 0) {
        AC_LOG_WARN(kModule, "Hex decode failed: odd length (%zu characters).", hex.size());
        return std::nullopt;
    }

    std::vector<std::uint8_t> out;
    out.reserve(hex.size() / 2);
    for (std::size_t i = 0; i < hex.size(); i += 2) {
        int hi = NibbleValue(hex[i]);
        int lo = NibbleValue(hex[i + 1]);
        if (hi < 0 || lo < 0) {
            AC_LOG_WARN(kModule, "Hex decode failed: invalid character at index %zu.", i);
            return std::nullopt;
        }
        out.push_back(static_cast<std::uint8_t>((hi << 4) | lo));
    }
    return out;
}

std::string Encode(std::span<const std::uint8_t> data) {
    static constexpr char kHex[] = "0123456789ABCDEF";
    std::string out;
    out.reserve(data.size() * 2);
    for (std::uint8_t b : data) {
        out.push_back(kHex[b >> 4]);
        out.push_back(kHex[b & 0x0F]);
    }
    return out;
}

}  // namespace ac::hex
