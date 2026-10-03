#include "pch.h"
#include "network/ManifestIdentity.h"

namespace ac::manifestidentity {

bool ReadU32Le(std::string_view bytes, std::size_t& offset, std::uint32_t& out) {
    if (bytes.size() - std::min(offset, bytes.size()) < 4) return false;
    const auto* p = reinterpret_cast<const unsigned char*>(bytes.data() + offset);
    out = static_cast<std::uint32_t>(p[0]) |
          (static_cast<std::uint32_t>(p[1]) << 8) |
          (static_cast<std::uint32_t>(p[2]) << 16) |
          (static_cast<std::uint32_t>(p[3]) << 24);
    offset += 4;
    return true;
}

bool ReadVarint(std::string_view bytes, std::size_t& offset, std::uint64_t& out) {
    out = 0;
    for (unsigned shift = 0; shift < 70; shift += 7) {
        if (offset >= bytes.size()) return false;
        const auto byte = static_cast<unsigned char>(bytes[offset++]);
        out |= static_cast<std::uint64_t>(byte & 0x7f) << shift;
        if ((byte & 0x80) == 0) return true;
    }
    return false;
}

bool ManifestIdentityMatches(std::string_view bytes, std::uint32_t expectedDepot,
                             std::uint64_t expectedGid) {
    std::size_t offset = 0;
    std::uint32_t magic = 0;
    std::uint32_t length = 0;
    if (!ReadU32Le(bytes, offset, magic) || !ReadU32Le(bytes, offset, length) ||
        magic != 0x71F617D0u || bytes.size() - offset < length) return false;
    offset += length;
    if (!ReadU32Le(bytes, offset, magic) || !ReadU32Le(bytes, offset, length) ||
        magic != 0x1F4812BEu || bytes.size() - offset < length) return false;

    const std::string_view metadata = bytes.substr(offset, length);
    std::size_t cursor = 0;
    std::uint32_t depot = 0;
    std::uint64_t gid = 0;
    while (cursor < metadata.size()) {
        std::uint64_t tag = 0;
        if (!ReadVarint(metadata, cursor, tag)) return false;
        const std::uint64_t field = tag >> 3;
        switch (tag & 7) {
        case 0: {
            std::uint64_t value = 0;
            if (!ReadVarint(metadata, cursor, value)) return false;
            if (field == 1) depot = static_cast<std::uint32_t>(value);
            if (field == 2) gid = value;
            break;
        }
        case 1:
            if (metadata.size() - cursor < 8) return false;
            cursor += 8;
            break;
        case 2: {
            std::uint64_t size = 0;
            if (!ReadVarint(metadata, cursor, size) || size > metadata.size() - cursor) return false;
            cursor += static_cast<std::size_t>(size);
            break;
        }
        case 5:
            if (metadata.size() - cursor < 4) return false;
            cursor += 4;
            break;
        default:
            return false;
        }
    }
    return depot == expectedDepot && gid == expectedGid;
}

}  // namespace ac::manifestidentity
