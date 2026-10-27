#include "pch.h"
#include "hooks/wire/SchemaCache.h"

#include <cstdint>
#include <string>

#include "hooks/wire/SchemaBuckets.h"
#include "utils/KeyValues.h"
#include "utils/WinFile.h"

namespace ac::hooks::wire::schemacache {
namespace {

constexpr const char* kSchemaPrefix = "UserGameStatsSchema_";
constexpr const char* kSchemaSuffix = ".bin";

std::string FileName(steam::AppId appId) { return std::string(kSchemaPrefix) + std::to_string(appId) + kSchemaSuffix; }

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

} // namespace

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

    // A cache entry Steam wrote itself is authoritative: donor data must never
    // overwrite a real download.
    if (!winfile::Exists(cachePath) && !winfile::WriteTextAtomically(cachePath, schema))
        return false;

    const std::string backupPath = Join(backupDir, name);
    if (!backupPath.empty() && !winfile::Exists(backupPath))
        winfile::WriteTextAtomically(backupPath, schema); // best effort: cache wins

    return true;
}

} // namespace ac::hooks::wire::schemacache
