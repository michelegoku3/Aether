#pragma once

#include <optional>
#include <string>
#include <string_view>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Parse Steam config-store / registry-style key path fragments.
// Shared by DepotHooks and DecryptionKeyHook (was duplicated verbatim).
// ---------------------------------------------------------------------------
namespace ac::keypath {

// From "...\\<depotId>\\DecryptionKey" → depotId. nullopt on malformation.
inline std::optional<steam::AppId> DepotIdFromDecryptionKeyName(const char* keyName) {
    if (!keyName) return std::nullopt;
    std::string key(keyName);
    const std::size_t marker = key.find("\\DecryptionKey");
    if (marker == std::string::npos || marker == 0) return std::nullopt;

    const std::size_t slash = key.find_last_of("\\/", marker - 1);
    const std::size_t start = (slash == std::string::npos) ? 0 : slash + 1;
    try {
        return static_cast<steam::AppId>(std::stoul(key.substr(start, marker - start)));
    } catch (...) {
        return std::nullopt;
    }
}

// From "apptickets\\<appId>" → appId. nullopt on malformation.
inline std::optional<steam::AppId> AppIdFromAppTicketKeyName(const char* keyName) {
    if (!keyName) return std::nullopt;
    std::string_view key(keyName);
    constexpr std::string_view prefix = "apptickets\\";
    if (key.size() <= prefix.size() || key.substr(0, prefix.size()) != prefix) {
        return std::nullopt;
    }
    key.remove_prefix(prefix.size());
    try {
        return static_cast<steam::AppId>(std::stoul(std::string(key)));
    } catch (...) {
        return std::nullopt;
    }
}

}  // namespace ac::keypath
