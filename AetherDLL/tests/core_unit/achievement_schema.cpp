#include "cases.h"

#include <cstdint>
#include <string>
#include <vector>

#include "hooks/wire/AchievementStatsSchema.h"

namespace {

// Minimal KV1 builder for the synthetic schemas below.
struct Bin {
    std::vector<std::uint8_t> bytes;

    Bin& Dict(const std::string& name) {
        bytes.push_back(0x00);
        Append(name);
        return *this;
    }
    Bin& Int(const std::string& key, std::int32_t value) {
        bytes.push_back(0x02);
        Append(key);
        for (int i = 0; i < 4; ++i)
            bytes.push_back(static_cast<std::uint8_t>((static_cast<std::uint32_t>(value) >> (8 * i)) & 0xFF));
        return *this;
    }
    Bin& Str(const std::string& key, const std::string& value) {
        bytes.push_back(0x01);
        Append(key);
        Append(value);
        return *this;
    }
    Bin& End(std::size_t count = 1) {
        for (std::size_t i = 0; i < count; ++i)
            bytes.push_back(0x08);
        return *this;
    }

  private:
    void Append(const std::string& text) {
        bytes.insert(bytes.end(), text.begin(), text.end());
        bytes.push_back(0x00);
    }
};

constexpr std::uint32_t Key(std::uint32_t bucket, std::uint32_t bit) { return bucket * 256u + bit; }

} // namespace

void AchievementSchema() {
    using ac::achieve::ParseStatsSchema;
    using ac::achieve::StatsSchema;

    // Mirrors the shape measured on the client: the stats section sits
    // under the appid section, achievement bits live in "<bucket>.bits", and the
    // counter names a stat declared far below.
    Bin schema;
    schema.Dict("1796470")
        .Dict("stats")
        .Dict("2")
        .Str("name", "STAT_2")
        .Dict("bits")
        .Dict("0")
        .Str("name", "ACH_00")
        .End()
        .Dict("1")
        .Str("name", "ACH_01")
        .Dict("progress")
        .Int("min_val", 0)
        .Int("max_val", 40)
        .Dict("value")
        .Str("operation", "statvalue")
        .Str("operand1", "STAT_7")
        .End()
        .End()
        .End()
        // No progress block: the achievement has no counter.
        .Dict("2")
        .Str("name", "ACH_02")
        .End()
        // An operation driven by another achievement carries no stat value.
        .Dict("3")
        .Str("name", "ACH_03")
        .Dict("progress")
        .Dict("value")
        .Str("operation", "achievement")
        .Str("operand1", "ACH_00")
        .End()
        .End()
        .End()
        // The counter names a stat the schema does not declare.
        .Dict("4")
        .Str("name", "ACH_04")
        .Dict("progress")
        .Dict("value")
        .Str("operation", "statvalue")
        .Str("operand1", "STAT_MISSING")
        .End()
        .End()
        .End()
        // A bit index past the 32-bit mask can never be read back.
        .Dict("33")
        .Str("name", "ACH_33")
        .Dict("progress")
        .Dict("value")
        .Str("operation", "statvalue")
        .Str("operand1", "STAT_7")
        .End()
        .End()
        .End()
        .End() // bits
        .End() // bucket 2
        .Dict("8")
        .Str("name", "STAT_7")
        .End()
        .End() // stats
        .Int("version", 5)
        .End()  // appid
        .End(); // stream

    StatsSchema parsed;
    CHECK(ParseStatsSchema(schema.bytes.data(), schema.bytes.size(), parsed));
    CHECK_EQ(parsed.progressStat.size(), 1u);
    CHECK(parsed.progressStat.contains(Key(2, 1)));
    CHECK(parsed.progressStat[Key(2, 1)] == 8u);
    CHECK(!parsed.progressStat.contains(Key(2, 0))); // no progress block
    CHECK(!parsed.progressStat.contains(Key(2, 2)));
    CHECK(!parsed.progressStat.contains(Key(2, 3)));  // other operation
    CHECK(!parsed.progressStat.contains(Key(2, 4)));  // unknown stat
    CHECK(!parsed.progressStat.contains(Key(2, 33))); // bit >= 32

    // A schema that keeps "stats" at the top level parses the same way.
    Bin flat;
    flat.Dict("stats")
        .Dict("5")
        .Dict("bits")
        .Dict("7")
        .Str("name", "ACH")
        .Dict("progress")
        .Dict("value")
        .Str("operation", "statvalue")
        .Str("operand1", "COUNTER")
        .End()
        .End()
        .End()
        .End()
        .End()
        .Dict("12")
        .Str("name", "COUNTER")
        .End()
        .End()
        .End();
    StatsSchema flatParsed;
    CHECK(ParseStatsSchema(flat.bytes.data(), flat.bytes.size(), flatParsed));
    CHECK(flatParsed.progressStat.size() == 1);
    CHECK(flatParsed.progressStat[Key(5, 7)] == 12u);

    // A schema with no achievement bits yields an empty map.
    Bin empty;
    empty.Dict("1").Dict("stats").Dict("3").Str("name", "S").End().End().End().End();
    StatsSchema emptyParsed;
    CHECK(ParseStatsSchema(empty.bytes.data(), empty.bytes.size(), emptyParsed));
    CHECK(emptyParsed.progressStat.empty());

    // A truncated stream is reported and leaves whatever was fully read. The
    // caller keeps answering with 0 progress rather than failing the page.
    StatsSchema truncated;
    CHECK(!ParseStatsSchema(schema.bytes.data(), 40, truncated));

    // No buffer at all: nothing to parse, no crash.
    StatsSchema none;
    CHECK(!ParseStatsSchema(nullptr, 0, none));
    CHECK(none.progressStat.empty());
}
