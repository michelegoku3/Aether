#pragma once

#include <cstdint>
#include <string>
#include <unordered_set>
#include <utility>
#include <vector>

#include "hooks/wire/AchievementUiPolicy.h"

// ---------------------------------------------------------------------------
// The gate's achievement schema, transcribed into Steam's own cache file.
//
// Two paths lead to a game's achievements, and they read different data:
//
//   * the client page (Player.GetGameAchievements#1) renders from the reply,
//     which carries every achievement with names, descriptions and icons;
//   * the overlay (Shift+Tab) and the game's own unlocks read
//     <steam>\appcache\stats\UserGameStatsSchema_<appid>.bin.
//
// For an app the account does not own, the CM answers the legacy request with
// k_EResultNoMatch and no schema, so the file is never written: the overlay's
// achievements tab stays disabled and Shift+Tab has nothing to list, while the
// library page shows the full list. A game whose owner is missing from the
// donor pool cannot be fixed by another donor, and every new release starts
// that way.
//
// The page reply already carries everything that file holds, so this encoder
// writes it. Nothing is invented: names, descriptions and icons come from
// Valve's reply; only the tree around them is ours.
//
// Tree (measured on files Steam writes itself, e.g. 1398210):
//
//   "<appid>" {
//       "stats" { "<bucket>" { "bits" { "<bit>" { "name" "<api>"
//                                                 "display" { ... } } }
//                              "type" "ACHIEVEMENTS" } }
//       "version"   <schema_version>
//       "gamename"  "<app title>"
//   }
//
// `internal_key` is `bucket * 256 + bit`, the same split the page rebuild uses,
// so an unlock recorded by the overlay lands on the bit the page lists. A bit
// past 31 cannot be read back (the stats file holds a 32-bit mask per bucket)
// and an entry with no API name can be neither matched nor unlocked: both are
// left out rather than written as dead rows.
//
// Pure: no disk, no logs, no proto, so the encoding is checked byte for byte.
// ---------------------------------------------------------------------------
namespace ac::achieve {

struct SchemaFileInput {
    std::uint32_t appId = 0;
    std::int32_t version = 0;         // the reply's schema_version
    std::string gameName;             // AppInfo title; omitted when AppInfo is not ready
    std::string language;             // language the names were captured in, "english" when empty
    std::vector<SchemaEntry> entries; // schema order, as the reply listed them
};

namespace detail {

// KV1 entry stream: a type byte, a NUL-terminated key, then the value.
inline void Dict(std::string& out, const std::string& name) {
    out.push_back('\x00');
    out += name;
    out.push_back('\0');
}

inline void Str(std::string& out, const std::string& key, const std::string& value) {
    out.push_back('\x01');
    out += key;
    out.push_back('\0');
    out += value;
    out.push_back('\0');
}

inline void Int(std::string& out, const std::string& key, std::int32_t value) {
    out.push_back('\x02');
    out += key;
    out.push_back('\0');
    const std::uint32_t raw = static_cast<std::uint32_t>(value);
    for (int byte = 0; byte < 4; ++byte)
        out.push_back(static_cast<char>((raw >> (8 * byte)) & 0xFFu));
}

inline void End(std::string& out, std::size_t count = 1) { out.append(count, '\x08'); }

// The languages a display block is written for: the capture language and
// english, so a later UI language switch still finds text instead of the raw
// NEW_ACHIEVEMENT_*_NAME token.
inline std::vector<std::string> Languages(const std::string& language) {
    if (language.empty() || language == "english")
        return {"english"};
    return {"english", language};
}

} // namespace detail

inline std::string EncodeSchemaFile(const SchemaFileInput& in) {
    // Group the reply's achievements into the file's bucket dictionaries,
    // keeping the reply's order: the first entry of a bucket creates it, and a
    // repeated key keeps the first one.
    std::vector<std::pair<std::uint32_t, std::vector<const SchemaEntry*>>> buckets;
    std::unordered_set<std::uint32_t> seen;
    for (const SchemaEntry& entry : in.entries) {
        const std::uint32_t bucket = entry.key >> 8;
        const std::uint32_t bit = entry.key & 0xFFu;
        if (entry.internalName.empty() || bit > 31u || !seen.insert(entry.key).second)
            continue;
        auto it = buckets.begin();
        while (it != buckets.end() && it->first != bucket)
            ++it;
        if (it == buckets.end())
            it = buckets.insert(buckets.end(), {bucket, {}});
        it->second.push_back(&entry);
    }
    if (buckets.empty())
        return {};

    const std::vector<std::string> languages = detail::Languages(in.language);
    std::string out;
    detail::Dict(out, std::to_string(in.appId));
    detail::Dict(out, "stats");
    for (const auto& [bucket, entries] : buckets) {
        const std::string bucketKey = std::to_string(bucket);
        detail::Dict(out, bucketKey);
        detail::Dict(out, "bits");
        for (const SchemaEntry* entry : entries) {
            const std::uint32_t bit = entry->key & 0xFFu;
            const std::string token = "NEW_ACHIEVEMENT_" + bucketKey + "_" + std::to_string(bit);
            detail::Dict(out, std::to_string(bit));
            detail::Str(out, "name", entry->internalName);
            detail::Dict(out, "display");
            detail::Dict(out, "name");
            for (const std::string& language : languages)
                detail::Str(out, language, entry->name);
            detail::Str(out, "token", token + "_NAME");
            detail::End(out);
            if (!entry->description.empty()) {
                detail::Dict(out, "desc");
                for (const std::string& language : languages)
                    detail::Str(out, language, entry->description);
                detail::Str(out, "token", token + "_DESC");
                detail::End(out);
            }
            detail::Int(out, "hidden", entry->hidden ? 1 : 0);
            if (!entry->icon.empty())
                detail::Str(out, "icon", entry->icon);
            if (!entry->iconGray.empty())
                detail::Str(out, "icon_gray", entry->iconGray);
            detail::End(out); // display
            detail::End(out); // bit
        }
        detail::End(out); // bits
        detail::Str(out, "type", "ACHIEVEMENTS");
        detail::End(out); // bucket
    }
    detail::End(out); // stats
    detail::Int(out, "version", in.version);
    if (!in.gameName.empty())
        detail::Str(out, "gamename", in.gameName);
    detail::End(out); // app id
    detail::End(out); // stream
    return out;
}

} // namespace ac::achieve
