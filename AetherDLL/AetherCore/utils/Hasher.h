#pragma once

#include <cstddef>
#include <cstdint>
#include <string>

// SHA-256 helper backed by the Windows CNG (bcrypt) API, so there is no
// third-party crypto dependency.
namespace ac::hasher {

// Streams filePath and returns the lowercase hex SHA-256 digest, or an empty
// string on any failure (logged).
std::string ComputeFileSha256(const std::string& filePath);

// FNV-1a 64 over an arbitrary buffer: cheap non-crypto fingerprint for log
// lines (e.g. "which schema version did this response carry?") without
// dumping the payload. Inline: header-only, no CMake changes needed.
inline std::uint64_t Fnv1a64(const void* data, std::size_t len) {
    const auto* p = static_cast<const unsigned char*>(data);
    std::uint64_t h = 14695981039346656037ull;
    for (std::size_t i = 0; i < len; ++i) {
        h ^= p[i];
        h *= 1099511628211ull;
    }
    return h;
}

}  // namespace ac::hasher
