#pragma once

// ---------------------------------------------------------------------------
// Which account Steam will sign in as, read from `config/loginusers.vdf`.
//
// The boot-time owner pass runs before Steam writes the live `ActiveUser`
// registry value. The account must still be the one that is about to sign in:
// taking the newest `userdata\<accountId>` folder instead picks whichever
// account Steam last touched locally (cloud sync, achievement backup), and
// stamps that stranger into every managed appmanifest right before login.
//
// Only `AutoLogin "1"` or, when nobody is configured for auto sign-in, the
// newest `timestamp` describe the next login. Header-only and pure so the
// matrix is unit-testable without Steam, the registry or a filesystem.
// ---------------------------------------------------------------------------
#include <cstdint>
#include <string>
#include <string_view>

#include "utils/VdfText.h"

namespace ac::steamid {

namespace loginusers_detail {

inline bool EqualsIgnoreCase(std::string_view a, std::string_view b) {
    if (a.size() != b.size())
        return false;
    for (std::size_t i = 0; i < a.size(); ++i) {
        char x = a[i];
        char y = b[i];
        if (x >= 'A' && x <= 'Z')
            x = static_cast<char>(x - 'A' + 'a');
        if (y >= 'A' && y <= 'Z')
            y = static_cast<char>(y - 'A' + 'a');
        if (x != y)
            return false;
    }
    return true;
}

inline bool ParseUnsigned(std::string_view text, std::uint64_t& out) {
    if (text.empty())
        return false;
    std::uint64_t value = 0;
    for (const char c : text) {
        if (c < '0' || c > '9')
            return false;
        const std::uint64_t digit = static_cast<std::uint64_t>(c - '0');
        if (value > (UINT64_MAX - digit) / 10)
            return false;
        value = value * 10 + digit;
    }
    out = value;
    return true;
}

// An account key is a decimal SteamID64 (individual base or later).
inline bool ParseSteamIdKey(std::string_view key, std::uint64_t& out) {
    if (key.size() != 17)
        return false;
    if (!ParseUnsigned(key, out))
        return false;
    return out >= 76561197960265728ull;
}

} // namespace loginusers_detail

// Returns the SteamID64 of the account Steam is configured to sign in as:
//   * the entry with `AutoLogin "1"`, else
//   * the entry with the newest `timestamp` (the last account used).
// 0 when the content has no usable `users` entry (missing, malformed or
// truncated). Never guesses from anything outside this file.
inline std::uint64_t SelectLoginAccount(std::string_view content) {
    struct Candidate {
        std::uint64_t steamId = 0;
        bool autoLogin = false;
        std::uint64_t timestamp = 0;
    };

    Candidate best;
    bool haveBest = false;
    auto consider = [&](const Candidate& candidate) {
        if (candidate.steamId == 0)
            return;
        const bool better = !haveBest || (candidate.autoLogin && !best.autoLogin) ||
                            (candidate.autoLogin == best.autoLogin && candidate.timestamp > best.timestamp);
        if (better) {
            best = candidate;
            haveBest = true;
        }
    };

    int depth = 0;
    std::string pendingKey; // last SectionKey, used when its `{` opens
    int pendingDepth = -1;
    std::string rootSection;
    bool inCandidate = false;
    Candidate current;

    std::size_t lineStart = 0;
    while (lineStart <= content.size()) {
        std::size_t lineEnd = content.find('\n', lineStart);
        if (lineEnd == std::string_view::npos)
            lineEnd = content.size();
        std::string line(content.substr(lineStart, lineEnd - lineStart));
        if (!line.empty() && line.back() == '\r')
            line.pop_back();

        std::size_t bracePos = 0;
        char brace = 0;
        if (vdf::FindUnquotedBrace(line, bracePos, brace)) {
            if (brace == '{') {
                ++depth;
                if (depth == 1) {
                    rootSection = pendingKey;
                } else if (depth == 2 && pendingDepth == 1 && rootSection == "users") {
                    std::uint64_t steamId = 0;
                    if (loginusers_detail::ParseSteamIdKey(pendingKey, steamId)) {
                        current = Candidate{};
                        current.steamId = steamId;
                        inCandidate = true;
                    }
                }
            } else {
                if (depth == 2 && inCandidate) {
                    consider(current);
                    inCandidate = false;
                }
                if (depth > 0)
                    --depth;
            }
        } else {
            std::string key;
            std::string value;
            const vdf::LineKind kind = vdf::ParseLine(line, key, value);
            if (kind == vdf::LineKind::SectionKey) {
                pendingKey = key;
                pendingDepth = depth;
            } else if (kind == vdf::LineKind::KeyValue && depth == 2 && inCandidate) {
                if (loginusers_detail::EqualsIgnoreCase(key, "AutoLogin")) {
                    current.autoLogin = value == "1";
                } else if (loginusers_detail::EqualsIgnoreCase(key, "timestamp")) {
                    std::uint64_t timestamp = 0;
                    if (loginusers_detail::ParseUnsigned(value, timestamp))
                        current.timestamp = timestamp;
                }
            }
        }

        if (lineEnd == content.size())
            break;
        lineStart = lineEnd + 1;
    }

    return haveBest ? best.steamId : 0;
}

} // namespace ac::steamid
