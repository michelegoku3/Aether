#include "cases.h"

#include "hooks/wire/SchemaCache.h"

namespace {

// Minimal but well-formed achievement schema: <appid> { stats { <bucket> {
// bits { "0" {} } } } }. Same shape the Steam client stores in
// appcache\stats\UserGameStatsSchema_<appid>.bin.
std::string BuildSchema(const std::string& root, const std::string& bucket, const std::string& child) {
    std::string data;

    const auto dict = [&data](const std::string& name) {
        data.push_back(0);
        data += name;
        data.push_back(0);
    };

    dict(root);
    dict("stats");
    dict(bucket);
    dict(child);
    dict("0");
    for (int i = 0; i < 5; ++i)
        data.push_back(8);
    return data;
}

std::string ReadAll(const fs::path& path) {
    std::ifstream f(path, std::ios::binary);
    return std::string((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
}

} // namespace

void SchemaCacheTest() {

    namespace sc = ac::hooks::wire::schemacache;

    // A blob with at least one achievement bucket is a usable schema.
    CHECK(sc::IsAchievementSchema(BuildSchema("4150860", "0", "bits")));

    // Garbage, empty, truncated and "stats" without any "bits" section are not.
    CHECK(!sc::IsAchievementSchema(""));
    CHECK(!sc::IsAchievementSchema("not a schema"));
    CHECK(!sc::IsAchievementSchema(BuildSchema("4150860", "0", "display")));
    CHECK(!sc::IsAchievementSchema(BuildSchema("4150860", "0", "bits").substr(0, 9)));

    Fixture fx;

    const fs::path statsDir = fx.dir / "appcache" / "stats";
    const fs::path backupDir = fx.dir / "backup" / "4150860" / "achievements";
    fs::create_directories(statsDir);
    fs::create_directories(backupDir);

    const std::string blob = BuildSchema("4150860", "0", "bits");
    const fs::path cacheFile = statsDir / "UserGameStatsSchema_4150860.bin";

    // An unusable blob never reaches Steam's cache.
    CHECK(!sc::Store(4150860, "garbage", statsDir.string(), backupDir.string()));
    CHECK(!fs::exists(cacheFile));

    // A schema whose root key names another app is refused: a mis-correlated
    // donor response must not poison this app's cache entry.
    CHECK(!sc::Store(4150860, BuildSchema("3764200", "0", "bits"), statsDir.string(), backupDir.string()));
    CHECK(!fs::exists(cacheFile));

    // A real schema lands byte-identical in the client cache and in the
    // AetherData backup, under Steam's own file name.
    CHECK(sc::Store(4150860, blob, statsDir.string(), backupDir.string()));
    CHECK(ReadAll(cacheFile) == blob);
    CHECK(ReadAll(backupDir / "UserGameStatsSchema_4150860.bin") == blob);

    // Staging file cleaned up: readers must never see a partial schema.
    CHECK(!fs::exists(cacheFile.string() + ".aether-write.tmp"));

    // An existing cache entry always wins (a real Steam download is never
    // replaced by a donor copy), and the call still reports a usable schema.
    {
        std::ofstream f(cacheFile, std::ios::binary | std::ios::trunc);
        f << "steam-own-copy";
    }
    CHECK(sc::Store(4150860, blob, statsDir.string(), backupDir.string()));
    CHECK(ReadAll(cacheFile) == "steam-own-copy");

    // No AetherData (empty backup dir): the cache write still succeeds.
    const std::string otherBlob = BuildSchema("4150861", "0", "bits");
    CHECK(sc::Store(4150861, otherBlob, statsDir.string(), ""));
    CHECK(ReadAll(statsDir / "UserGameStatsSchema_4150861.bin") == otherBlob);

    // Unusable destination: reported, not thrown.
    CHECK(!sc::Store(4150862, BuildSchema("4150862", "0", "bits"), (fx.dir / "missing" / "stats").string(), ""));
}
