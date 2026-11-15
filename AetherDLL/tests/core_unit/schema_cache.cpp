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

void SchemaCacheRestore() {
    namespace sc = ac::hooks::wire::schemacache;

    Fixture fx;
    const fs::path statsDir = fx.dir / "appcache" / "stats";
    fs::create_directories(statsDir);

    const auto write = [](const fs::path& path, const std::string& bytes) {
        fs::create_directories(path.parent_path());
        std::ofstream f(path, std::ios::binary | std::ios::trunc);
        f << bytes;
    };
    const auto read = [](const fs::path& path) {
        std::ifstream f(path, std::ios::binary);
        return std::string((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
    };

    // The client builds its achievement list at login from the cache file, so a
    // session that starts before the page was opened gets the transcribed
    // schema back from Aether's own copy.
    const std::string blob = BuildSchema("4150863", "0", "bits");
    write(fx.dir / "backup" / "4150863" / "achievements" / "UserGameStatsSchema_4150863.bin", blob);
    CHECK(sc::Restore(4150863, statsDir.string(), (fx.dir / "backup" / "4150863" / "achievements").string()));
    CHECK(read(statsDir / "UserGameStatsSchema_4150863.bin") == blob);

    // Nothing to restore without a backup copy.
    CHECK(!sc::Restore(4150864, statsDir.string(), (fx.dir / "backup" / "4150864" / "achievements").string()));
    CHECK(!fs::exists(statsDir / "UserGameStatsSchema_4150864.bin"));

    // A file Steam wrote is never replaced, even when the backup differs.
    write(statsDir / "UserGameStatsSchema_4150865.bin", "steam-own-copy");
    write(fx.dir / "backup" / "4150865" / "achievements" / "UserGameStatsSchema_4150865.bin",
          BuildSchema("4150865", "0", "bits"));
    CHECK(sc::Restore(4150865, statsDir.string(), (fx.dir / "backup" / "4150865" / "achievements").string()));
    CHECK(read(statsDir / "UserGameStatsSchema_4150865.bin") == "steam-own-copy");

    // A backup copy that names another app, or that does not parse, is refused
    // instead of published under this app's file name.
    write(fx.dir / "backup" / "4150866" / "achievements" / "UserGameStatsSchema_4150866.bin",
          BuildSchema("3764200", "0", "bits"));
    CHECK(!sc::Restore(4150866, statsDir.string(), (fx.dir / "backup" / "4150866" / "achievements").string()));
    write(fx.dir / "backup" / "4150867" / "achievements" / "UserGameStatsSchema_4150867.bin", "garbage");
    CHECK(!sc::Restore(4150867, statsDir.string(), (fx.dir / "backup" / "4150867" / "achievements").string()));
    CHECK(!fs::exists(statsDir / "UserGameStatsSchema_4150866.bin"));
    CHECK(!fs::exists(statsDir / "UserGameStatsSchema_4150867.bin"));

    // The restore target must be a real directory: a missing stats dir is
    // reported, never created from here.
    CHECK(!sc::Restore(4150863, (fx.dir / "missing" / "stats").string(),
                       (fx.dir / "backup" / "4150863" / "achievements").string()));

    // App id from the backup directory name, which is how the startup pass
    // finds what to restore; anything else yields 0 (nothing to do).
    CHECK_EQ(sc::AppIdFromBackupDir("4809710"), 4809710u);
    CHECK_EQ(sc::AppIdFromBackupDir("570"), 570u);
    CHECK_EQ(sc::AppIdFromBackupDir(""), 0u);
    CHECK_EQ(sc::AppIdFromBackupDir("0"), 0u);
    CHECK_EQ(sc::AppIdFromBackupDir("abc"), 0u);
    CHECK_EQ(sc::AppIdFromBackupDir("4809710.bin"), 0u);
    CHECK_EQ(sc::AppIdFromBackupDir("-4809710"), 0u);
    CHECK_EQ(sc::AppIdFromBackupDir("99999999999"), 0u);
}
