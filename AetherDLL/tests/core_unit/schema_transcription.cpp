#include "cases.h"

#include <cstdint>
#include <string>

#include "hooks/wire/SchemaCache.h"
#include "utils/Hasher.h"

namespace {

// Minimal well-formed achievement schema: <appid> { stats { <bucket> {
// bits { "0" {} } } } }, the shape the client stores in appcache\stats.
std::string BuildSchema(const std::string& root, const std::string& bucket) {
    std::string data;
    const auto dict = [&data](const std::string& name) {
        data.push_back(0);
        data += name;
        data.push_back(0);
    };
    dict(root);
    dict("stats");
    dict(bucket);
    dict("bits");
    dict("0");
    for (int i = 0; i < 5; ++i)
        data.push_back(8);
    return data;
}

std::string ReadAll(const fs::path& path) {
    std::ifstream f(path, std::ios::binary);
    return std::string((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
}

void Write(const fs::path& path, const std::string& bytes) {
    fs::create_directories(path.parent_path());
    std::ofstream f(path, std::ios::binary | std::ios::trunc);
    f << bytes;
}

} // namespace

void SchemaTranscription() {
    namespace sc = ac::hooks::wire::schemacache;
    using sc::DecideWrite;
    using sc::SchemaWrite;
    using sc::TranscribedSchema;

    // The marker is a small text file next to the backup copy: it records what
    // Aether wrote, so a later capture can tell "our file" from "Steam's file".
    TranscribedSchema marker;
    marker.present = true;
    marker.version = 5;
    marker.count = 42;
    marker.hash = 0x42DE091B25E79CE2ull;
    const std::string text = sc::EncodeTranscribed(marker);
    TranscribedSchema parsed;
    CHECK(sc::ParseTranscribed(text, parsed));
    CHECK(parsed.present);
    CHECK_EQ(parsed.version, 5);
    CHECK_EQ(parsed.count, 42u);
    CHECK_EQ(parsed.hash, 0x42DE091B25E79CE2ull);

    // Anything that is not that file is "no marker", never a guess.
    TranscribedSchema garbage;
    CHECK(!sc::ParseTranscribed("", garbage));
    CHECK(!sc::ParseTranscribed("UserGameStatsSchema_1.bin", garbage));
    CHECK(!sc::ParseTranscribed("aether-transcribed 1\nversion\n", garbage));
    CHECK(!sc::ParseTranscribed("aether-transcribed 1\nversion four\ncount 1\nfnv1a 0\n", garbage));

    // Line endings from a Windows editor (CRLF) are the same file.
    TranscribedSchema crlf;
    CHECK(sc::ParseTranscribed("aether-transcribed 1\r\nversion 5\r\ncount 42\r\nfnv1a 42de091b25e79ce2\r\n", crlf));
    CHECK_EQ(crlf.version, 5);
    CHECK_EQ(crlf.count, 42u);
    CHECK_EQ(crlf.hash, 0x42DE091B25E79CE2ull);

    // Ownership: the same bytes as when the marker was written, nothing else.
    const std::string ours = BuildSchema("4150870", "0");
    TranscribedSchema written = marker;
    written.hash = ac::hasher::Fnv1a64(ours.data(), ours.size());
    CHECK(sc::IsTranscribedFile(written, ours));
    CHECK(!sc::IsTranscribedFile(written, BuildSchema("4150870", "1")));
    CHECK(!sc::IsTranscribedFile(written, ""));
    CHECK(!sc::IsTranscribedFile(TranscribedSchema{}, ours)); // no marker at all

    // Decision table. A missing cache is always written; a file that is not
    // ours never is; ours is rewritten only when the capture is newer.
    CHECK(DecideWrite(written, "", 1, 1) == SchemaWrite::Write);               // nothing cached yet
    CHECK(DecideWrite(TranscribedSchema{}, ours, 9, 99) == SchemaWrite::Skip); // Steam's file
    CHECK(DecideWrite(written, ours, 5, 42) == SchemaWrite::Skip);             // already current
    CHECK(DecideWrite(written, ours, 4, 42) == SchemaWrite::Skip);             // older capture
    CHECK(DecideWrite(written, ours, 6, 42) == SchemaWrite::Write);            // new schema version
    CHECK(DecideWrite(written, ours, 5, 43) == SchemaWrite::Write);            // same version, more entries
    CHECK(DecideWrite(marker, ours, 9, 99) == SchemaWrite::Skip);              // hash does not match

    Fixture fx;
    const fs::path statsDir = fx.dir / "appcache" / "stats";
    const fs::path backupDir = fx.dir / "backup" / "4150870" / "achievements";
    fs::create_directories(statsDir);
    fs::create_directories(backupDir);
    const fs::path cacheFile = statsDir / "UserGameStatsSchema_4150870.bin";
    const fs::path backupFile = backupDir / "UserGameStatsSchema_4150870.bin";
    const fs::path markerFile = backupDir / "UserGameStatsSchema_4150870.transcribed";
    const std::string dir = statsDir.string();
    const std::string backup = backupDir.string();

    // First transcription: file, backup copy and marker appear together, so the
    // next capture knows the file is Aether's.
    CHECK(sc::StoreTranscribed(4150870, ours, dir, backup, 5, 42));
    CHECK(ReadAll(cacheFile) == ours);
    CHECK(ReadAll(backupFile) == ours);
    TranscribedSchema stored;
    CHECK(sc::ParseTranscribed(ReadAll(markerFile), stored));
    CHECK_EQ(stored.version, 5);
    CHECK_EQ(stored.count, 42u);
    CHECK_EQ(stored.hash, ac::hasher::Fnv1a64(ours.data(), ours.size()));

    // Same capture again: nothing to do, and the cache keeps its bytes.
    const auto stamp = fs::last_write_time(cacheFile);
    CHECK(sc::StoreTranscribed(4150870, ours, dir, backup, 5, 42));
    CHECK_EQ(fs::last_write_time(cacheFile), stamp);

    // The game adds achievements: a newer capture replaces our file, the backup
    // and the marker, so the overlay lists the new set.
    const std::string grown = BuildSchema("4150870", "1");
    CHECK(sc::StoreTranscribed(4150870, grown, dir, backup, 6, 60));
    CHECK(ReadAll(cacheFile) == grown);
    CHECK(ReadAll(backupFile) == grown);
    CHECK(sc::ParseTranscribed(ReadAll(markerFile), stored));
    CHECK_EQ(stored.version, 6);
    CHECK_EQ(stored.count, 60u);

    // A file Steam wrote (same name, other bytes, no marker) is never replaced,
    // whatever the capture says — and the capture cannot reclaim it either.
    Write(cacheFile, "steam-own-copy");
    CHECK(sc::StoreTranscribed(4150870, BuildSchema("4150870", "2"), dir, backup, 7, 99));
    CHECK(ReadAll(cacheFile) == "steam-own-copy");
    CHECK(ReadAll(backupFile) == grown); // the backup is Aether's copy of its own write

    // A donor that finally owns the game brings the real schema: it replaces our
    // transcription (richer: counters and every language) and drops the marker,
    // because the file is no longer ours.
    const fs::path donorDir = fx.dir / "appcache" / "stats";
    const fs::path donorBackup = fx.dir / "backup" / "4150872" / "achievements";
    fs::create_directories(donorBackup);
    const std::string donor = donorBackup.string();
    const std::string oursForDonor = BuildSchema("4150872", "1");
    CHECK(sc::StoreTranscribed(4150872, oursForDonor, dir, donor, 6, 60));
    const fs::path donorCache = donorDir / "UserGameStatsSchema_4150872.bin";
    const fs::path donorMarker = donorBackup / "UserGameStatsSchema_4150872.transcribed";
    CHECK(ReadAll(donorCache) == oursForDonor);
    CHECK(fs::exists(donorMarker));
    const std::string real = BuildSchema("4150872", "3");
    CHECK(sc::Store(4150872, real, dir, donor));
    CHECK(ReadAll(donorCache) == real);
    CHECK(!fs::exists(donorMarker));
    // And our capture cannot overwrite it any more.
    CHECK(sc::StoreTranscribed(4150872, BuildSchema("4150872", "4"), dir, donor, 9, 99));
    CHECK(ReadAll(donorCache) == real);

    // A restore from the backup keeps ownership: marker and bytes come back
    // together, so a later update still works.
    const fs::path restoreBackup = fx.dir / "backup" / "4150873" / "achievements";
    fs::create_directories(restoreBackup);
    const std::string restoreDirB = restoreBackup.string();
    CHECK(sc::StoreTranscribed(4150873, BuildSchema("4150873", "1"), dir, restoreDirB, 6, 60));
    const fs::path restoreCache = donorDir / "UserGameStatsSchema_4150873.bin";
    const fs::path restoreMarker = restoreBackup / "UserGameStatsSchema_4150873.transcribed";
    fs::remove(restoreCache);
    CHECK(sc::Restore(4150873, dir, restoreDirB));
    CHECK(ReadAll(restoreCache) == BuildSchema("4150873", "1"));
    CHECK(fs::exists(restoreMarker));
    CHECK(sc::StoreTranscribed(4150873, BuildSchema("4150873", "5"), dir, restoreDirB, 7, 70));
    CHECK(ReadAll(restoreCache) == BuildSchema("4150873", "5"));

    // Nothing worth writing: garbage or another app's blob is refused, and an
    // app with no backup dir still gets the cache file (written once, no marker).
    CHECK(!sc::StoreTranscribed(4150870, "garbage", dir, backup, 9, 99));
    CHECK(!sc::StoreTranscribed(4150870, BuildSchema("3764200", "0"), dir, backup, 9, 99));
    CHECK(sc::StoreTranscribed(4150871, BuildSchema("4150871", "0"), dir, "", 1, 1));
    CHECK(ReadAll(statsDir / "UserGameStatsSchema_4150871.bin") == BuildSchema("4150871", "0"));
}
