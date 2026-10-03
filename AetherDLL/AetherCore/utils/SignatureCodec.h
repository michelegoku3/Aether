#pragma once

// ---------------------------------------------------------------------------
// IDA-style hex signature parser ("48 8B 05 ?? ?? ?? ?? C3").
//
// Estratto dal namespace anonimo di PatternEngine.cpp: la logica è pura
// (nessun I/O, nessuno stato globale) e qui vive in un header inline così
// può essere testata in isolamento dalla suite "signature" di quickwin_tests.
// ---------------------------------------------------------------------------

#include <cstdint>
#include <sstream>
#include <string>
#include <vector>

#include "core/Logger.h"

namespace ac::pattern {

// Parsa una signature hex separata da spazi in una sequenza di byte + una
// maschera di pari lunghezza ('x' = byte significativo, '?' = wildcard).
// Ritorna false su token malformati o input vuoto. I log di errore usano il
// modulo "PatternEngine" per continuità con i log storici.
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
        }
        else {
            try {
                if (token.size() != 2) {
                    AC_LOG_WARN(kSigModule, "Bad signature token '%s'.", token.c_str());
                    return false;
                }
                std::size_t consumed = 0;
                unsigned long value = std::stoul(token, &consumed, 16);
                if (consumed != token.size() || value > 0xFFul) {
                    AC_LOG_WARN(kSigModule, "Bad signature token '%s'.", token.c_str());
                    return false;
                }
                bytes.push_back(static_cast<std::uint8_t>(value));
                mask.push_back('x');
            }
            catch (...) {
                AC_LOG_WARN(kSigModule, "Bad signature token '%s'.", token.c_str());
                return false;
            }
        }
    }
    return !bytes.empty();
}

}  // namespace ac::pattern
