#include "pch.h"
#include "network/ManifestIdentity.h"

#include <limits>

namespace ac::manifestidentity {

bool ReadU32Le(std::string_view bytes, std::size_t& offset, std::uint32_t& out) {
    if (bytes.size() - std::min(offset, bytes.size()) < 4)
        return false;
    const auto* p = reinterpret_cast<const unsigned char*>(bytes.data() + offset);
    out = static_cast<std::uint32_t>(p[0]) | (static_cast<std::uint32_t>(p[1]) << 8) |
          (static_cast<std::uint32_t>(p[2]) << 16) | (static_cast<std::uint32_t>(p[3]) << 24);
    offset += 4;
    return true;
}

bool ReadVarint(std::string_view bytes, std::size_t& offset, std::uint64_t& out) {
    out = 0;
    for (unsigned shift = 0; shift < 70; shift += 7) {
        if (offset >= bytes.size())
            return false;
        const auto byte = static_cast<unsigned char>(bytes[offset++]);
        out |= static_cast<std::uint64_t>(byte & 0x7f) << shift;
        if ((byte & 0x80) == 0)
            return true;
    }
    return false;
}

bool ManifestIdentityMatches(std::string_view bytes, std::uint32_t expectedDepot, std::uint64_t expectedGid) {
    std::size_t offset = 0;
    std::uint32_t magic = 0;
    std::uint32_t length = 0;
    if (!ReadU32Le(bytes, offset, magic) || !ReadU32Le(bytes, offset, length) || magic != 0x71F617D0u ||
        bytes.size() - offset < length)
        return false;
    offset += length;
    if (!ReadU32Le(bytes, offset, magic) || !ReadU32Le(bytes, offset, length) || magic != 0x1F4812BEu ||
        bytes.size() - offset < length)
        return false;

    const std::string_view metadata = bytes.substr(offset, length);
    std::size_t cursor = 0;
    std::uint32_t depot = 0;
    std::uint64_t gid = 0;
    while (cursor < metadata.size()) {
        std::uint64_t tag = 0;
        if (!ReadVarint(metadata, cursor, tag))
            return false;
        const std::uint64_t field = tag >> 3;
        switch (tag & 7) {
        case 0: {
            std::uint64_t value = 0;
            if (!ReadVarint(metadata, cursor, value))
                return false;
            if (field == 1)
                depot = static_cast<std::uint32_t>(value);
            if (field == 2)
                gid = value;
            break;
        }
        case 1:
            if (metadata.size() - cursor < 8)
                return false;
            cursor += 8;
            break;
        case 2: {
            std::uint64_t size = 0;
            if (!ReadVarint(metadata, cursor, size) || size > metadata.size() - cursor)
                return false;
            cursor += static_cast<std::size_t>(size);
            break;
        }
        case 5:
            if (metadata.size() - cursor < 4)
                return false;
            cursor += 4;
            break;
        default:
            return false;
        }
    }
    return depot == expectedDepot && gid == expectedGid;
}

namespace {

// One `ContentManifestFile` entry: field 2 is the file size. Chunk metadata and
// filenames are skipped; an absent size is an empty file.
bool FileEntrySize(std::string_view entry, std::uint64_t& size) {
    size = 0;
    std::size_t cursor = 0;
    while (cursor < entry.size()) {
        std::uint64_t tag = 0;
        if (!ReadVarint(entry, cursor, tag))
            return false;
        const std::uint64_t field = tag >> 3;
        switch (tag & 7) {
        case 0: {
            std::uint64_t value = 0;
            if (!ReadVarint(entry, cursor, value))
                return false;
            if (field == 2)
                size = value;
            break;
        }
        case 1:
            if (entry.size() - cursor < 8)
                return false;
            cursor += 8;
            break;
        case 2: {
            std::uint64_t length = 0;
            if (!ReadVarint(entry, cursor, length) || length > entry.size() - cursor)
                return false;
            cursor += static_cast<std::size_t>(length);
            break;
        }
        case 5:
            if (entry.size() - cursor < 4)
                return false;
            cursor += 4;
            break;
        default:
            return false;
        }
    }
    return true;
}

} // namespace

bool ManifestContentSize(std::string_view bytes, std::uint64_t& out) {
    out = 0;
    std::size_t offset = 0;
    std::uint32_t magic = 0;
    std::uint32_t length = 0;
    if (!ReadU32Le(bytes, offset, magic) || !ReadU32Le(bytes, offset, length) || magic != 0x71F617D0u ||
        bytes.size() - offset < length)
        return false;

    const std::string_view payload = bytes.substr(offset, length);
    std::size_t cursor = 0;
    std::uint64_t total = 0;
    while (cursor < payload.size()) {
        std::uint64_t tag = 0;
        if (!ReadVarint(payload, cursor, tag))
            return false;
        const std::uint64_t field = tag >> 3;
        switch (tag & 7) {
        case 0: {
            std::uint64_t value = 0;
            if (!ReadVarint(payload, cursor, value))
                return false;
            break;
        }
        case 1:
            if (payload.size() - cursor < 8)
                return false;
            cursor += 8;
            break;
        case 2: {
            std::uint64_t entryLength = 0;
            if (!ReadVarint(payload, cursor, entryLength) || entryLength > payload.size() - cursor)
                return false;
            const std::string_view entry = payload.substr(cursor, static_cast<std::size_t>(entryLength));
            cursor += static_cast<std::size_t>(entryLength);
            if (field == 1) {
                std::uint64_t fileSize = 0;
                if (!FileEntrySize(entry, fileSize))
                    return false;
                // Saturating, like the AetherDesk twin: a manifest is never
                // exabytes long, and a wrapped sum would silently look valid.
                total = fileSize > std::numeric_limits<std::uint64_t>::max() - total
                            ? std::numeric_limits<std::uint64_t>::max()
                            : total + fileSize;
            }
            break;
        }
        case 5:
            if (payload.size() - cursor < 4)
                return false;
            cursor += 4;
            break;
        default:
            return false;
        }
    }
    out = total;
    return true;
}

} // namespace ac::manifestidentity
