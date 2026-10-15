#pragma once

// ---------------------------------------------------------------------------
// IDA-style hex signature parser ("48 8B 05 ?? ?? ?? ?? C3").
//
// Extracted from PatternEngine.cpp's anonymous namespace: the logic is pure
// (no I/O, no global state) and lives here in an inline header so it can be
// tested in isolation by core_unit_tests' "signature" suite.
// ---------------------------------------------------------------------------

#include <cstdint>
#include <sstream>
#include <string>
#include <vector>

#include "core/Logger.h"

namespace ac::pattern {

// Parses a space-separated hex signature into a byte sequence + an
// equal-length mask ('x' = significant byte, '?' = wildcard). Returns false
// on malformed tokens or empty input. Error logs use the "PatternEngine"
// module for continuity with historic logs.
inline bool ParseSignature(const std::string& sig, std::vector<std::uint8_t>& bytes, std::string& mask) {
    constexpr const char* kSigModule = "PatternEngine";
    bytes.clear();
    mask.clear();
    std::istringstream iss(sig);
    std::string token;
    while (iss >> token) {
        if (token == "??") {
            bytes.push_back(0);
            mask.push_back('?');
        } else {
            try {
                if (token.size() != 2) {
                    AC_LOG_WARN(kSigModule, "Bad signature token '%s'.", token.c_str());
                    return false;
                }
                std::size_t consumed = 0;
                unsigned long value = std::stoul(token, &consumed, 16);
                if (consumed != token.size() || value > 0xFFull) {
                    AC_LOG_WARN(kSigModule, "Bad signature token '%s'.", token.c_str());
                    return false;
                }
                bytes.push_back(static_cast<std::uint8_t>(value));
                mask.push_back('x');
            } catch (...) {
                AC_LOG_WARN(kSigModule, "Bad signature token '%s'.", token.c_str());
                return false;
            }
        }
    }
    return !bytes.empty();
}

} // namespace ac::pattern
