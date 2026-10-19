#pragma once

#include <cstdint>
#include <limits>
#include <string>
#include <string_view>

#include "utils/Strings.h"
#include "utils/VdfText.h"

// ---------------------------------------------------------------------------
// appmanifest text editing.
//
// Steam stores the account that last wrote an app's state as the top-level
// `AppState.LastOwner` entry in `steamapps/appmanifest_<app>.acf`. Steam's own
// UI treats that value as the app's owner: for an app the current account does
// not subscribe to, the library renders "From the library of <owner>" whenever
// the recorded owner is a different account (see sp.js
// `#AppDetails_InSharedLibrary`). A leftover LastOwner from another login
// therefore leaves a bogus shared-library banner on Lua-provided apps.
//
// Two policies share these helpers:
//   * remove the entry (default): account-independent, the banner cannot
//     render for any login because Steam has nobody to attribute the app to;
//   * set it to a fixed account: opt-in cosmetic banner for selected appids.
//
// Both rewrite only that single value/line: byte-for-byte preservation of
// indentation, key casing and line endings, so Steam's parser sees an
// otherwise identical file.
//
// Header-only and pure (no logging, no filesystem): testable without Win32.
// ---------------------------------------------------------------------------
namespace ac::appmanifest {

// Span of the value token in a `"key" "value"` line (offsets exclude quotes).
inline bool FindValueSpan(std::string_view line, std::size_t& begin, std::size_t& length) {
    std::size_t i = 0;
    auto skipSpace = [&] {
        while (i < line.size() && (line[i] == ' ' || line[i] == '\t'))
            ++i;
    };
    auto readToken = [&](std::size_t& tokenBegin, std::size_t& tokenLength) {
        skipSpace();
        if (i >= line.size() || line[i] != '"')
            return false;
        ++i;
        tokenBegin = i;
        const std::size_t end = line.find('"', i);
        if (end == std::string_view::npos)
            return false;
        tokenLength = end - i;
        i = end + 1;
        return true;
    };
    std::size_t keyBegin = 0;
    std::size_t keyLength = 0;
    return readToken(keyBegin, keyLength) && readToken(begin, length);
}

// Sets the first top-level `"LastOwner"` value to `ownerSteamId`, inserting the
// key after `"appid"` when the appmanifest has none. Returns true when the body
// changed. A non-AppState body or a zero identity is a no-op.
inline bool SetLastOwner(std::string& content, std::uint64_t ownerSteamId) {
    if (ownerSteamId == 0)
        return false;
    const std::string wanted = std::to_string(ownerSteamId);

    int depth = 0;
    std::size_t lineStart = 0;
    std::size_t insertAt = std::string::npos;
    bool insertUnterminated = false;
    std::string lineEnding = "\n";

    while (lineStart < content.size()) {
        const std::size_t newline = content.find('\n', lineStart);
        const std::size_t lineStop = (newline == std::string::npos) ? content.size() : newline;
        std::string line = content.substr(lineStart, lineStop - lineStart);
        bool carriageReturn = false;
        if (!line.empty() && line.back() == '\r') {
            carriageReturn = true;
            line.pop_back();
        }

        std::string key;
        std::string value;
        const vdf::LineKind kind = vdf::ParseLine(line, key, value);
        if (kind == vdf::LineKind::KeyValue) {
            if (depth == 1 && ac::strings::EqualsIgnoreCase(key, "LastOwner")) {
                if (value == wanted)
                    return false;
                std::size_t valueStart = 0;
                std::size_t valueLength = 0;
                if (!FindValueSpan(line, valueStart, valueLength))
                    return false;
                content.replace(lineStart + valueStart, valueLength, wanted);
                return true;
            }
            if (depth == 1 && ac::strings::EqualsIgnoreCase(key, "appid") && insertAt == std::string::npos) {
                insertAt = (newline == std::string::npos) ? content.size() : newline + 1;
                insertUnterminated = (newline == std::string::npos);
                lineEnding = carriageReturn ? "\r\n" : "\n";
            }
        } else {
            bool quoted = false;
            for (const char c : line) {
                if (c == '"')
                    quoted = !quoted;
                else if (!quoted && c == '{')
                    ++depth;
                else if (!quoted && c == '}')
                    --depth;
            }
        }

        if (newline == std::string::npos)
            break;
        lineStart = newline + 1;
    }

    if (insertAt == std::string::npos)
        return false;

    const std::string entry = "\t\"LastOwner\"\t\t\"" + wanted + "\"";
    if (insertUnterminated)
        content += lineEnding + entry;
    else
        content.insert(insertAt, entry + lineEnding);
    return true;
}

// Removes the first top-level `"LastOwner"` line, preserving every other byte
// (indentation, key casing, line endings). Returns true when the body changed.
inline bool RemoveLastOwner(std::string& content) {
    int depth = 0;
    std::size_t lineStart = 0;

    while (lineStart < content.size()) {
        const std::size_t newline = content.find('\n', lineStart);
        const std::size_t lineStop = (newline == std::string::npos) ? content.size() : newline;

        std::string line = content.substr(lineStart, lineStop - lineStart);
        if (!line.empty() && line.back() == '\r')
            line.pop_back();

        std::string key;
        std::string value;
        if (vdf::ParseLine(line, key, value) == vdf::LineKind::KeyValue) {
            if (depth == 1 && ac::strings::EqualsIgnoreCase(key, "LastOwner")) {
                const std::size_t eraseEnd = (newline == std::string::npos) ? lineStop : newline + 1;
                content.erase(lineStart, eraseEnd - lineStart);
                return true;
            }
        } else {
            bool quoted = false;
            for (const char c : line) {
                if (c == '"')
                    quoted = !quoted;
                else if (!quoted && c == '{')
                    ++depth;
                else if (!quoted && c == '}')
                    --depth;
            }
        }

        if (newline == std::string::npos)
            break;
        lineStart = newline + 1;
    }
    return false;
}

// Value of the first top-level `"LastOwner"`, or 0 when absent or not a decimal
// uint64. Lets the cleanup policy tell the value Aether wrote from one Steam
// has since rewritten (for example after a real family license changed hands).
inline std::uint64_t ReadLastOwner(std::string_view content) {
    int depth = 0;
    std::size_t lineStart = 0;

    while (lineStart < content.size()) {
        const std::size_t newline = content.find('\n', lineStart);
        const std::size_t lineStop = (newline == std::string::npos) ? content.size() : newline;

        std::string line(content.substr(lineStart, lineStop - lineStart));
        if (!line.empty() && line.back() == '\r')
            line.pop_back();

        std::string key;
        std::string value;
        if (vdf::ParseLine(line, key, value) == vdf::LineKind::KeyValue) {
            if (depth == 1 && ac::strings::EqualsIgnoreCase(key, "LastOwner")) {
                std::uint64_t parsed = 0;
                if (value.empty())
                    return 0;
                for (const char c : value) {
                    if (c < '0' || c > '9')
                        return 0;
                    const std::uint64_t digit = static_cast<std::uint64_t>(c - '0');
                    if (parsed > (std::numeric_limits<std::uint64_t>::max() - digit) / 10)
                        return 0;
                    parsed = parsed * 10 + digit;
                }
                return parsed;
            }
        } else {
            bool quoted = false;
            for (const char c : line) {
                if (c == '"')
                    quoted = !quoted;
                else if (!quoted && c == '{')
                    ++depth;
                else if (!quoted && c == '}')
                    --depth;
            }
        }

        if (newline == std::string::npos)
            break;
        lineStart = newline + 1;
    }
    return 0;
}

} // namespace ac::appmanifest
