#pragma once

#include <charconv>
#include <cstdint>
#include <map>
#include <string>
#include <string_view>

// ============================================================================
// AchievementPrefetch — when Aether asks the CM for a game's achievement
// catalog on its own.
//
// The client fixes an app's schema before its first GetUserStats request of a
// session and does not re-read the file afterwards, so a catalog written later
// in that session only helps the next start. Asking for the catalog at startup
// closes the gap: the file is there before the client looks.
//
// Two reasons not to ask: the schema file already exists (a Steam download or an
// earlier transcription), or the CM already answered "no achievements" for this
// app recently. That answer is remembered in a hint file and retried after
// kEmptyAnswerTtlSeconds, because an update can add achievements to a game that
// had none.
// ============================================================================
namespace ac::hooks::wire::prefetch {

// How long a "no achievements" answer is trusted before asking again.
inline constexpr std::uint64_t kEmptyAnswerTtlSeconds = 7ull * 24ull * 60ull * 60ull;

// "<appid> <unix>" per app, sorted by id.
using Markers = std::map<std::uint32_t, std::uint64_t>;

// Reads the hint file. Unknown or malformed lines are dropped: the file is
// user-visible and editable, so one bad line must never hide the rest.
inline Markers ParseMarkers(const std::string& text) {
    Markers out;
    std::size_t start = 0;
    while (start < text.size()) {
        std::size_t end = text.find('\n', start);
        if (end == std::string::npos)
            end = text.size();
        std::string_view line(text.data() + start, end - start);
        start = end + 1;
        if (!line.empty() && line.back() == '\r')
            line.remove_suffix(1);
        const std::size_t space = line.find(' ');
        if (space == std::string_view::npos)
            continue;
        std::uint32_t appId = 0;
        std::uint64_t at = 0;
        const auto [appEnd, appError] = std::from_chars(line.data(), line.data() + space, appId);
        if (appError != std::errc{} || appEnd != line.data() + space || appId == 0)
            continue;
        const auto [atEnd, atError] = std::from_chars(line.data() + space + 1, line.data() + line.size(), at);
        if (atError != std::errc{} || atEnd != line.data() + line.size() || at == 0)
            continue;
        out[appId] = at; // a later line for the same app wins
    }
    return out;
}

// Sorted output: the file is meant to be read and diffed by a human.
inline std::string EncodeMarkers(const Markers& markers) {
    std::string out;
    for (const auto& [appId, at] : markers) {
        out += std::to_string(appId);
        out.push_back(' ');
        out += std::to_string(at);
        out.push_back('\n');
    }
    return out;
}

// True while a recorded empty answer is still trusted. A clock that moved
// backwards (at > now) is not trusted: asking again is always safe.
inline bool IsAnswerFresh(std::uint64_t at, std::uint64_t now) {
    if (at == 0 || now < at)
        return false;
    return now - at < kEmptyAnswerTtlSeconds;
}

// Ask only what no schema file covers and no fresh empty answer explains.
inline bool ShouldAsk(bool schemaFileExists, bool answerFresh) { return !schemaFileExists && !answerFresh; }

} // namespace ac::hooks::wire::prefetch
