#include "cases.h"

#include <cstdint>
#include <map>
#include <string>
#include <vector>

#include "hooks/wire/AchievementSchemaExport.h"
#include "hooks/wire/AchievementStatsSchema.h"
#include "hooks/wire/SchemaCache.h"
#include "utils/KeyValues.h"

namespace {

// Byte-level oracle: the tree below is what Steam writes for the same data
// (app id root, "stats", one dictionary per bucket with "bits", then "version"
// and "gamename"). Built by hand so it cannot inherit the encoder's mistakes.
struct Bin {
    std::string bytes;

    Bin& Dict(const std::string& name) {
        bytes.push_back(0x00);
        Append(name);
        return *this;
    }
    Bin& Int(const std::string& key, std::int32_t value) {
        bytes.push_back(0x02);
        Append(key);
        for (int i = 0; i < 4; ++i)
            bytes.push_back(static_cast<char>((static_cast<std::uint32_t>(value) >> (8 * i)) & 0xFF));
        return *this;
    }
    Bin& Str(const std::string& key, const std::string& value) {
        bytes.push_back(0x01);
        Append(key);
        Append(value);
        return *this;
    }
    Bin& End(std::size_t count = 1) {
        bytes.append(count, static_cast<char>(0x08));
        return *this;
    }

  private:
    void Append(const std::string& text) {
        bytes += text;
        bytes.push_back('\0');
    }
};

// Path-addressed view of a schema: "424242/stats/2/bits/3/name" -> value.
struct Flatten final : ac::kv1::Visitor {
    std::vector<std::string> stack;
    std::map<std::string, std::string> strings;
    std::map<std::string, std::int32_t> ints;

    std::string Path(const std::string& key) const {
        std::string out;
        for (const std::string& part : stack)
            out += part + "/";
        return out + key;
    }
    void OnDictBegin(const std::string& name, int) override { stack.push_back(name); }
    void OnDictEnd(int) override {
        if (!stack.empty())
            stack.pop_back();
    }
    void OnString(const std::string& key, const std::string& value, int) override { strings[Path(key)] = value; }
    void OnInt32(const std::string& key, std::uint8_t, std::int32_t value, int) override { ints[Path(key)] = value; }
};

Flatten Flattened(const std::string& bytes) {
    Flatten out;
    CHECK(ac::kv1::WalkBinary(reinterpret_cast<const std::uint8_t*>(bytes.data()), bytes.size(), out));
    return out;
}

ac::achieve::SchemaEntry Entry(std::uint32_t key, std::string api, std::string name, std::string description,
                               std::string icon, std::string iconGray, bool hidden) {
    ac::achieve::SchemaEntry entry;
    entry.key = key;
    entry.internalName = std::move(api);
    entry.name = std::move(name);
    entry.description = std::move(description);
    entry.icon = std::move(icon);
    entry.iconGray = std::move(iconGray);
    entry.hidden = hidden;
    return entry;
}

// A capture of app 424242, version 7, Italian names.
ac::achieve::SchemaFileInput Input(std::vector<ac::achieve::SchemaEntry> entries) {
    ac::achieve::SchemaFileInput in;
    in.appId = 424242;
    in.version = 7;
    in.gameName = "Test Game";
    in.language = "italian";
    in.entries = std::move(entries);
    return in;
}

} // namespace

void AchievementExport() {
    using ac::achieve::EncodeSchemaFile;

    // One achievement, every field filled: the whole tree, byte for byte.
    // Names/descriptions are stored under the language the capture was made in
    // and under "english", so a later UI language switch still shows text.
    Bin one;
    one.Dict("424242")
        .Dict("stats")
        .Dict("2")
        .Dict("bits")
        .Dict("3")
        .Str("name", "ACH_3")
        .Dict("display")
        .Dict("name")
        .Str("english", "Traguardo")
        .Str("italian", "Traguardo")
        .Str("token", "NEW_ACHIEVEMENT_2_3_NAME")
        .End()
        .Dict("desc")
        .Str("english", "Fatto")
        .Str("italian", "Fatto")
        .Str("token", "NEW_ACHIEVEMENT_2_3_DESC")
        .End()
        .Int("hidden", 1)
        .Str("icon", "a.jpg")
        .Str("icon_gray", "b.jpg")
        .End() // display
        .End() // bit 3
        .End() // bits
        .Str("type", "ACHIEVEMENTS")
        .End() // bucket 2
        .End() // stats
        .Int("version", 7)
        .Str("gamename", "Test Game")
        .End()  // app id
        .End(); // stream

    const std::string encoded =
        EncodeSchemaFile(Input({Entry(2 * 256 + 3, "ACH_3", "Traguardo", "Fatto", "a.jpg", "b.jpg", true)}));
    CHECK_EQ(encoded, one.bytes);

    // The production readers accept it: one achievement bucket, no counters,
    // and the cache writer's own validation (root key == app id) passes.
    CHECK(ac::hooks::wire::schemacache::IsAchievementSchema(encoded));
    ac::achieve::StatsSchema stats;
    CHECK(ac::achieve::ParseStatsSchema(reinterpret_cast<const std::uint8_t*>(encoded.data()), encoded.size(), stats));
    CHECK(stats.progressStat.empty());

    // Achievements of one bucket share a single bucket dictionary; a second
    // bucket gets its own. Buckets appear in the order the reply lists them.
    Bin merged;
    merged.Dict("424242")
        .Dict("stats")
        .Dict("2")
        .Dict("bits")
        .Dict("0")
        .Str("name", "A0")
        .Dict("display")
        .Dict("name")
        .Str("english", "Zero")
        .Str("italian", "Zero")
        .Str("token", "NEW_ACHIEVEMENT_2_0_NAME")
        .End()
        .Int("hidden", 0)
        .End()
        .End()
        .Dict("3")
        .Str("name", "A3")
        .Dict("display")
        .Dict("name")
        .Str("english", "Three")
        .Str("italian", "Three")
        .Str("token", "NEW_ACHIEVEMENT_2_3_NAME")
        .End()
        .Int("hidden", 0)
        .End()
        .End()
        .End()
        .Str("type", "ACHIEVEMENTS")
        .End()
        .Dict("5")
        .Dict("bits")
        .Dict("1")
        .Str("name", "B1")
        .Dict("display")
        .Dict("name")
        .Str("english", "Uno")
        .Str("italian", "Uno")
        .Str("token", "NEW_ACHIEVEMENT_5_1_NAME")
        .End()
        .Int("hidden", 0)
        .End()
        .End()
        .End()
        .Str("type", "ACHIEVEMENTS")
        .End()
        .End()
        .Int("version", 7)
        .Str("gamename", "Test Game")
        .End()
        .End();

    CHECK_EQ(EncodeSchemaFile(Input({Entry(2 * 256 + 0, "A0", "Zero", "", "", "", false),
                                     Entry(5 * 256 + 1, "B1", "Uno", "", "", "", false),
                                     Entry(2 * 256 + 3, "A3", "Three", "", "", "", false)})),
             merged.bytes);

    // Round trip through the reader: every field is where the client looks.
    const Flatten flat = Flattened(encoded);
    CHECK_EQ(flat.strings.at("424242/stats/2/bits/3/name"), std::string("ACH_3"));
    CHECK_EQ(flat.strings.at("424242/stats/2/bits/3/display/name/english"), std::string("Traguardo"));
    CHECK_EQ(flat.strings.at("424242/stats/2/bits/3/display/name/italian"), std::string("Traguardo"));
    CHECK_EQ(flat.strings.at("424242/stats/2/bits/3/display/desc/english"), std::string("Fatto"));
    CHECK_EQ(flat.strings.at("424242/stats/2/bits/3/display/icon"), std::string("a.jpg"));
    CHECK_EQ(flat.strings.at("424242/stats/2/bits/3/display/icon_gray"), std::string("b.jpg"));
    CHECK_EQ(flat.strings.at("424242/stats/2/bits/3/display/name/token"), std::string("NEW_ACHIEVEMENT_2_3_NAME"));
    CHECK_EQ(flat.strings.at("424242/stats/2/type"), std::string("ACHIEVEMENTS"));
    CHECK_EQ(flat.strings.at("424242/gamename"), std::string("Test Game"));
    CHECK_EQ(flat.ints.at("424242/stats/2/bits/3/display/hidden"), 1);
    CHECK_EQ(flat.ints.at("424242/version"), 7);

    // An English capture writes a single "english" key, and a capture with no
    // language at all is treated as English.
    ac::achieve::SchemaFileInput english = Input({Entry(1 * 256 + 0, "A", "Name", "", "", "", false)});
    english.language = "english";
    const Flatten englishFlat = Flattened(EncodeSchemaFile(english));
    CHECK_EQ(englishFlat.strings.at("424242/stats/1/bits/0/display/name/english"), std::string("Name"));
    CHECK_EQ(englishFlat.strings.size(), 5u); // api name, display name, token, bucket type, gamename

    ac::achieve::SchemaFileInput noLanguage = Input({Entry(1 * 256 + 0, "A", "Name", "", "", "", false)});
    noLanguage.language.clear();
    const Flatten noLanguageFlat = Flattened(EncodeSchemaFile(noLanguage));
    CHECK_EQ(noLanguageFlat.strings.at("424242/stats/1/bits/0/display/name/english"), std::string("Name"));
    CHECK_EQ(noLanguageFlat.strings.size(), 5u);

    // Nothing to transcribe: no file at all, rather than a file with no
    // achievements.
    CHECK(EncodeSchemaFile(Input({})).empty());

    // An entry without an API name can be neither matched nor unlocked, and a
    // key past the 32-bit bucket mask can never be read back: both are skipped.
    // A duplicate key keeps the first entry.
    const Flatten skipped =
        Flattened(EncodeSchemaFile(Input({Entry(3 * 256 + 0, "", "No API", "", "", "", false),
                                          Entry(3 * 256 + 32, "PAST_MASK", "Past", "", "", "", false),
                                          Entry(3 * 256 + 1, "DUP", "First", "", "", "", false),
                                          Entry(3 * 256 + 1, "DUP2", "Second", "", "", "", false)})));
    CHECK_EQ(skipped.strings.at("424242/stats/3/bits/1/name"), std::string("DUP"));
    CHECK(!skipped.strings.contains("424242/stats/3/bits/32/name"));
    CHECK(!skipped.strings.contains("424242/stats/3/bits/0/name"));
    CHECK_EQ(skipped.strings.size(), 6u); // api name, english, italian, token, bucket type, gamename

    // Missing description and icons are left out instead of written empty, and
    // a capture with no game name still produces a usable schema.
    ac::achieve::SchemaFileInput plain = Input({Entry(1 * 256 + 0, "A", "Name", "", "", "", false)});
    plain.gameName.clear();
    const std::string plainBytes = EncodeSchemaFile(plain);
    const Flatten plainFlat = Flattened(plainBytes);
    CHECK(!plainFlat.strings.contains("424242/stats/1/bits/0/display/desc/english"));
    CHECK(!plainFlat.strings.contains("424242/stats/1/bits/0/display/icon"));
    CHECK(!plainFlat.strings.contains("424242/gamename"));
    CHECK(ac::hooks::wire::schemacache::IsAchievementSchema(plainBytes));
}
