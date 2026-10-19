#pragma once

#include <cstdint>
#include <string>

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
// The value cannot be corrected with "the current account": Steam loads app
// state at startup, before it knows which account will log in (ActiveUser,
// loginusers.vdf and AutoLoginUser are all written at login time). Any stamp is
// therefore a guess about the *previous* session, which merely flips the banner
// to the other account on every switch.
//
// Removing the entry is account-independent: Steam then has no owner to
// attribute, so the banner cannot render for any login
// (`strOwnerSteamID` invalid -> `return null` in sp.js). The appmanifest format
// tolerates its absence (e.g. appmanifest_480.acf ships without it).
//
// Header-only and pure (no logging, no filesystem): testable without Win32.
// ---------------------------------------------------------------------------
namespace ac::appmanifest {

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

} // namespace ac::appmanifest
