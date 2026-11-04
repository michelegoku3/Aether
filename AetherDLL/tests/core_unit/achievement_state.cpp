#include "cases.h"

#include <cstdint>
#include <string>
#include <vector>

#include "hooks/wire/AchievementUserState.h"

namespace {

// UserGameStats_1916127674_3618390.bin: one achievement bucket (1) with 24 of
// its 27 bits set and a date for every set bit.
constexpr std::uint8_t kGamblersTable[] = {
    0x00, 0x63, 0x61, 0x63, 0x68, 0x65, 0x00, 0x02, 0x63, 0x72, 0x63, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x50, 0x65,
    0x6E, 0x64, 0x69, 0x6E, 0x67, 0x43, 0x68, 0x61, 0x6E, 0x67, 0x65, 0x73, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x31,
    0x00, 0x02, 0x64, 0x61, 0x74, 0x61, 0x00, 0xFF, 0xFF, 0xBF, 0x02, 0x02, 0x73, 0x74, 0x61, 0x74, 0x65, 0x00, 0x02,
    0x00, 0x00, 0x00, 0x02, 0x70, 0x65, 0x6E, 0x64, 0x69, 0x6E, 0x67, 0x62, 0x69, 0x74, 0x73, 0x00, 0xFF, 0xFF, 0xBF,
    0x02, 0x00, 0x41, 0x63, 0x68, 0x69, 0x65, 0x76, 0x65, 0x6D, 0x65, 0x6E, 0x74, 0x54, 0x69, 0x6D, 0x65, 0x73, 0x00,
    0x02, 0x30, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x31, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x32, 0x00, 0x73, 0x25,
    0xE3, 0x6A, 0x02, 0x33, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x34, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x35, 0x00,
    0x73, 0x25, 0xE3, 0x6A, 0x02, 0x36, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x37, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02,
    0x38, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x39, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x31, 0x30, 0x00, 0x73, 0x25,
    0xE3, 0x6A, 0x02, 0x31, 0x31, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x31, 0x32, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02,
    0x31, 0x33, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x31, 0x34, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x31, 0x35, 0x00,
    0x73, 0x25, 0xE3, 0x6A, 0x02, 0x31, 0x36, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x31, 0x37, 0x00, 0x73, 0x25, 0xE3,
    0x6A, 0x02, 0x31, 0x38, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x31, 0x39, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x32,
    0x30, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x32, 0x31, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x02, 0x32, 0x33, 0x00, 0x73,
    0x25, 0xE3, 0x6A, 0x02, 0x32, 0x35, 0x00, 0x73, 0x25, 0xE3, 0x6A, 0x08, 0x08, 0x08, 0x08,
};

constexpr std::uint32_t kGamblersTime = 0x6AE32573u;

// Minimal KV1 builder for the synthetic cases.
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

} // namespace

void AchievementState() {
    ac::achieve::UserState state;
    CHECK(ac::achieve::ParseUserState(kGamblersTable, sizeof(kGamblersTable), state));
    CHECK(state.unlocked.size() == 24);
    // key = bucket * 256 + bit; bucket 1, bits 0..21, 23, 25.
    CHECK(state.unlocked.contains(256u));
    CHECK(state.unlocked[256u] == kGamblersTime);
    CHECK(state.unlocked[256u + 21u] == kGamblersTime);
    CHECK(!state.unlocked.contains(256u + 22u));
    CHECK(state.unlocked[256u + 23u] == kGamblersTime);
    CHECK(!state.unlocked.contains(256u + 24u));
    CHECK(state.unlocked[256u + 25u] == kGamblersTime);
    CHECK(!state.unlocked.contains(256u + 26u)); // bit 26 is clear in 0x02BFFFFF

    // A truncated file keeps the rows it managed to read: the bitfield is read
    // before the dates, so the unlock keys survive with time 0.
    ac::achieve::UserState partial;
    CHECK(!ac::achieve::ParseUserState(kGamblersTable, 52, partial));
    CHECK(partial.unlocked.size() == 24);
    CHECK(partial.unlocked[256u] == 0u);

    // A high bucket number and a bucket without dates.
    Bin high;
    high.Dict("cache").Dict("358").Int("data", 1).End(2).End();
    ac::achieve::UserState highState;
    CHECK(ac::achieve::ParseUserState(high.bytes.data(), high.bytes.size(), highState));
    CHECK(highState.unlocked.size() == 1);
    CHECK(highState.unlocked[358u * 256u] == 0u); // 91648

    // An unconfirmed unlock lives in "pendingbits": for an app the account does
    // not own the server never acks the bit, so it never reaches "data".
    Bin pendingOnly;
    pendingOnly.Dict("cache").Dict("2").Int("pendingbits", 0x8).End(2).End();
    ac::achieve::UserState pendingState;
    CHECK(ac::achieve::ParseUserState(pendingOnly.bytes.data(), pendingOnly.bytes.size(), pendingState));
    CHECK(pendingState.unlocked.size() == 1);
    CHECK(pendingState.unlocked[512u + 3u] == 0u);

    // Both masks in one bucket: the union, each key once.
    Bin both;
    both.Dict("cache").Dict("1").Int("data", 0x1).Int("pendingbits", 0x1).End(2).End();
    ac::achieve::UserState bothState;
    CHECK(ac::achieve::ParseUserState(both.bytes.data(), both.bytes.size(), bothState));
    CHECK(bothState.unlocked.size() == 1);
    CHECK(bothState.unlocked[256u] == 0u);

    // A plain stat bucket (name/type, no "data") is not an achievement bucket.
    Bin stats;
    stats.Dict("cache").Dict("24").Str("name", "num_matches").Str("type", "INT").Int("default", 0).End(2).End();
    ac::achieve::UserState statsState;
    CHECK(ac::achieve::ParseUserState(stats.bytes.data(), stats.bytes.size(), statsState));
    CHECK(statsState.empty());

    // No key in the file at all.
    Bin other;
    other.Dict("cache").Int("crc", 0).End();
    ac::achieve::UserState otherState;
    CHECK(ac::achieve::ParseUserState(other.bytes.data(), other.bytes.size(), otherState));
    CHECK(otherState.empty());
}
