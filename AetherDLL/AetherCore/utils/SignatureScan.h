#pragma once

#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

namespace ac::pattern {

enum class ScanStatus {
    Unique,
    NotFound,
    Ambiguous,
    Invalid,
};

struct ScanResult {
    ScanStatus status = ScanStatus::NotFound;
    std::size_t offset = 0;
};

// Scans haystack[0..haystackLen) for bytes constrained by mask ('x' = exact
// match, any other character = wildcard). Returns Unique only when exactly one
// match exists so callers never hook a duplicated stub or mid-block fragment.
inline ScanResult FindUniqueSignatureMatch(const std::uint8_t* haystack, std::size_t haystackLen,
                                           const std::vector<std::uint8_t>& bytes, const std::string& mask) {
    if (!haystack || bytes.empty() || mask.size() != bytes.size()) {
        return {ScanStatus::Invalid, 0};
    }
    if (haystackLen < bytes.size()) {
        return {ScanStatus::NotFound, 0};
    }

    const std::size_t limit = haystackLen - bytes.size();
    const bool checkFirst = (mask[0] == 'x');
    const std::uint8_t firstByte = bytes[0];

    bool found = false;
    std::size_t foundOffset = 0;

    for (std::size_t i = 0; i <= limit; ++i) {
        if (checkFirst && haystack[i] != firstByte) {
            continue;
        }
        bool matched = true;
        for (std::size_t j = 0; j < bytes.size(); ++j) {
            if (mask[j] == 'x' && haystack[i + j] != bytes[j]) {
                matched = false;
                break;
            }
        }
        if (!matched) {
            continue;
        }
        if (found) {
            return {ScanStatus::Ambiguous, 0};
        }
        found = true;
        foundOffset = i;
    }

    if (!found) {
        return {ScanStatus::NotFound, 0};
    }
    return {ScanStatus::Unique, foundOffset};
}

} // namespace ac::pattern
