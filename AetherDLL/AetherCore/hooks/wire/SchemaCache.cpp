#include "pch.h"
#include "hooks/wire/SchemaCache.h"

#include <charconv>
#include <cstdint>
#include <fstream>
#include <iterator>
#include <string>

#include "hooks/wire/SchemaBuckets.h"
#include "utils/Hasher.h"
#include "utils/KeyValues.h"
#include "utils/WinFile.h"

namespace ac::hooks::wire::schemacache {
namespace {

constexpr const char* kSchemaPrefix = "UserGameStatsSchema_";
constexpr const char* kSchemaSuffix = ".bin";
constexpr const char* kMarkerSuffix = ".transcribed";
// First line of a marker, with the format number it was written in.
constexpr const char* kMarkerHeader = "aether-transcribed 1";

std::string FileName(steam::AppId appId) { return std::string(kSchemaPrefix) + std::to_string(appId) + kSchemaSuffix; }

std::string MarkerName(steam::AppId appId) {
    return std::string(kSchemaPrefix) + std::to_string(appId) + kMarkerSuffix;
}

// Join without dragging in std::filesystem: both roots come from the caller
// already trimmed (Steam install root, AetherData backup tree).
std::string Join(const std::string& dir, const std::string& name) {
    if (dir.empty())
        return {};
    const char last = dir.back();
    return (last == '\\' || last == '/') ? dir + name : dir + "\\" + name;
}

// The schema's root key IS the app id (verified against Steam's own downloads:
// 00 "3764200" 00 "stats" ...). A blob for another app must never be written
// under this app's file name: a mis-correlated donor response would otherwise
// poison the cache entry.
bool RootIsAppId(const std::string& schema, steam::AppId appId) {
    if (schema.size() < 3 || static_cast<std::uint8_t>(schema[0]) != 0)
        return false;
    const std::size_t nul = schema.find('\0', 1);
    if (nul == std::string::npos)
        return false;
    return schema.compare(1, nul - 1, std::to_string(appId)) == 0;
}

bool ReadFile(const std::string& path, std::string& out) {
    std::ifstream file(winfile::ToPath(path), std::ios::binary);
    if (!file.is_open())
        return false;
    out.assign(std::istreambuf_iterator<char>(file), std::istreambuf_iterator<char>());
    return true;
}

std::string Hex(std::uint64_t value) {
    static constexpr char kDigits[] = "0123456789abcdef";
    std::string out(16, '0');
    for (int nibble = 15; nibble >= 0; --nibble) {
        out[static_cast<std::size_t>(nibble)] = kDigits[value & 0xFull];
        value >>= 4;
    }
    return out;
}

// One "key value" line of a marker; false when the line is malformed.
bool ParseUint(std::string_view value, std::uint64_t& out) {
    if (value.empty())
        return false;
    const auto [end, error] = std::from_chars(value.data(), value.data() + value.size(), out, 10);
    return error == std::errc{} && end == value.data() + value.size();
}

bool ParseSigned(std::string_view value, std::int32_t& out) {
    if (value.empty())
        return false;
    const auto [end, error] = std::from_chars(value.data(), value.data() + value.size(), out, 10);
    return error == std::errc{} && end == value.data() + value.size();
}

bool ParseHex64(std::string_view text, std::uint64_t& out) {
    if (text.size() != 16)
        return false;
    const auto [end, error] = std::from_chars(text.data(), text.data() + text.size(), out, 16);
    return error == std::errc{} && end == text.data() + text.size();
}

// Whether Aether may publish over whatever is in the cache now. Reading is
// best effort: an unreadable marker means "not ours", which is the safe side.
TranscribedSchema ReadMarker(steam::AppId appId, const std::string& backupDir) {
    TranscribedSchema marker;
    if (backupDir.empty())
        return marker;
    std::string text;
    if (!ReadFile(Join(backupDir, MarkerName(appId)), text))
        return marker;
    ParseTranscribed(text, marker);
    return marker;
}

void RemoveMarker(steam::AppId appId, const std::string& backupDir) {
    if (backupDir.empty())
        return;
    const bool removed = winfile::Delete(Join(backupDir, MarkerName(appId)));
    (void)removed; // a marker that is already gone needs no error handling
}

bool WriteMarker(steam::AppId appId, const std::string& backupDir, const TranscribedSchema& marker) {
    if (backupDir.empty())
        return true; // no backup tree: nothing to remember, the cache write stands
    return winfile::WriteTextAtomically(Join(backupDir, MarkerName(appId)), EncodeTranscribed(marker));
}

} // namespace

std::string EncodeTranscribed(const TranscribedSchema& marker) {
    std::string text = kMarkerHeader;
    text += "\nversion " + std::to_string(marker.version);
    text += "\ncount " + std::to_string(marker.count);
    text += "\nfnv1a " + Hex(marker.hash);
    text += "\n";
    return text;
}

bool ParseTranscribed(const std::string& text, TranscribedSchema& out) {
    out = TranscribedSchema{};
    std::size_t lineStart = 0;
    const auto nextLine = [&text, &lineStart]() -> std::string_view {
        if (lineStart >= text.size())
            return {};
        const std::size_t end = text.find('\n', lineStart);
        std::string_view line(text.data() + lineStart, (end == std::string::npos ? text.size() : end) - lineStart);
        lineStart = end == std::string::npos ? text.size() : end + 1;
        // A file rewritten by a Windows editor carries CRLF: same marker.
        if (!line.empty() && line.back() == '\r')
            line.remove_suffix(1);
        return line;
    };

    if (nextLine() != kMarkerHeader)
        return false;
    bool haveVersion = false;
    bool haveCount = false;
    bool haveHash = false;
    for (std::string_view line = nextLine(); !line.empty(); line = nextLine()) {
        const std::size_t space = line.find(' ');
        if (space == std::string_view::npos)
            return false;
        const std::string_view key = line.substr(0, space);
        const std::string_view value = line.substr(space + 1);
        if (key == "version") {
            if (!ParseSigned(value, out.version))
                return false;
            haveVersion = true;
        } else if (key == "count") {
            std::uint64_t count = 0;
            if (!ParseUint(value, count) || count > 0xFFFFFFFFull)
                return false;
            out.count = static_cast<std::uint32_t>(count);
            haveCount = true;
        } else if (key == "fnv1a") {
            if (!ParseHex64(value, out.hash))
                return false;
            haveHash = true;
        } else {
            return false; // unknown field: a newer format, do not guess
        }
    }
    if (!haveVersion || !haveCount || !haveHash)
        return false;
    out.present = true;
    return true;
}

bool IsTranscribedFile(const TranscribedSchema& marker, const std::string& cacheBytes) {
    if (!marker.present || cacheBytes.empty())
        return false;
    return marker.hash == ac::hasher::Fnv1a64(cacheBytes.data(), cacheBytes.size());
}

SchemaWrite DecideWrite(const TranscribedSchema& marker, const std::string& cacheBytes, std::int32_t version,
                        std::uint32_t count) {
    if (cacheBytes.empty())
        return SchemaWrite::Write; // nothing there yet
    if (!IsTranscribedFile(marker, cacheBytes))
        return SchemaWrite::Skip; // Steam's file, or one another donor published
    if (version > marker.version)
        return SchemaWrite::Write;
    // A game that does not bump its schema version but adds achievements is
    // still worth the rewrite: more rows is strictly more information.
    if (version == marker.version && count > marker.count)
        return SchemaWrite::Write;
    return SchemaWrite::Skip;
}

bool IsAchievementSchema(const std::string& schema) {
    if (schema.size() < 4)
        return false;
    ac::backup::statscache::detail::SchemaBucketWalker walker;
    const bool parsed =
        ac::kv1::WalkBinary(reinterpret_cast<const std::uint8_t*>(schema.data()), schema.size(), walker);
    // A stats section without a single "bits" bucket defines no achievement:
    // writing it would leave the game with an empty achievement list anyway.
    return parsed && !walker.buckets.empty();
}

bool Store(steam::AppId appId, const std::string& schema, const std::string& statsDir, const std::string& backupDir) {
    if (!IsAchievementSchema(schema) || !RootIsAppId(schema, appId))
        return false;

    const std::string name = FileName(appId);
    const std::string cachePath = Join(statsDir, name);
    if (cachePath.empty())
        return false;

    std::string current;
    const bool cached = ReadFile(cachePath, current);
    // A cache entry Steam wrote itself is authoritative: donor data must never
    // overwrite a real download. Aether's own transcription is not: the real
    // schema carries counters and every language, so it takes over.
    if (cached && !IsTranscribedFile(ReadMarker(appId, backupDir), current))
        return true;
    if (!winfile::WriteTextAtomically(cachePath, schema))
        return false;
    RemoveMarker(appId, backupDir); // the file is no longer Aether's

    const std::string backupPath = Join(backupDir, name);
    if (!backupPath.empty())
        winfile::WriteTextAtomically(backupPath, schema); // best effort: cache wins

    return true;
}

bool StoreTranscribed(steam::AppId appId, const std::string& schema, const std::string& statsDir,
                      const std::string& backupDir, std::int32_t version, std::uint32_t count, SchemaWrite* applied) {
    if (!IsAchievementSchema(schema) || !RootIsAppId(schema, appId))
        return false;

    const std::string name = FileName(appId);
    const std::string cachePath = Join(statsDir, name);
    if (cachePath.empty())
        return false;

    std::string current;
    if (!ReadFile(cachePath, current))
        current.clear(); // absent or unreadable: DecideWrite treats it as nothing cached
    const SchemaWrite decision = DecideWrite(ReadMarker(appId, backupDir), current, version, count);
    if (applied)
        *applied = decision;
    if (decision == SchemaWrite::Skip)
        return true; // a usable file is already in place

    if (!winfile::WriteTextAtomically(cachePath, schema))
        return false;

    const std::string backupPath = Join(backupDir, name);
    if (!backupPath.empty())
        winfile::WriteTextAtomically(backupPath, schema); // best effort: the cache is what counts

    TranscribedSchema marker;
    marker.present = true;
    marker.version = version;
    marker.count = count;
    marker.hash = ac::hasher::Fnv1a64(schema.data(), schema.size());
    const bool marked = WriteMarker(appId, backupDir, marker);
    (void)marked; // best effort: without it a later capture only stops updating

    return true;
}

bool Restore(steam::AppId appId, const std::string& statsDir, const std::string& backupDir) {
    const std::string name = FileName(appId);
    const std::string cachePath = Join(statsDir, name);
    if (cachePath.empty())
        return false;
    if (winfile::Exists(cachePath))
        return true; // Steam's own copy, or one an earlier run put back

    const std::string backupPath = Join(backupDir, name);
    if (backupPath.empty())
        return false;
    std::string bytes;
    if (!ReadFile(backupPath, bytes))
        return false;
    // Same two checks as a fresh write: the file must be a schema and it must be
    // this app's, so a stale copy can never be published under another app.
    if (!IsAchievementSchema(bytes) || !RootIsAppId(bytes, appId))
        return false;
    if (!winfile::WriteTextAtomically(cachePath, bytes))
        return false;
    // The marker already sits next to the backup copy and names these bytes, so
    // ownership survives the restore without a second write here.
    return true;
}

steam::AppId AppIdFromBackupDir(const std::string& name) {
    if (name.empty() || name.size() > 10)
        return 0; // longer than any uint32 app id
    std::uint64_t value = 0;
    for (const char c : name) {
        if (c < '0' || c > '9')
            return 0;
        value = value * 10 + static_cast<std::uint64_t>(c - '0');
        if (value > 0xFFFFFFFFull)
            return 0;
    }
    return static_cast<steam::AppId>(value); // 0 for "0", i.e. no app
}

} // namespace ac::hooks::wire::schemacache
