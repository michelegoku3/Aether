#include "pch.h"
#include "hooks/wire/AchievementUserState.h"

#include <charconv>
#include <cstring>
#include <string>

#include "utils/KeyValues.h"

namespace ac::achieve {
namespace {

constexpr std::uint8_t kTypeFloat = 0x03;

// UserGameStats_<account>_<appid>.bin (KV1):
//
//   "cache" {
//       <bucket> { "data" <value or bitfield> "pendingbits" <bitfield>
//                  "AchievementTimes" { <bit> <time> } }
//       ...
//   }
//
// `data` is the unlock bitfield of an achievement bucket and the current value
// of a stat bucket: the schema reply decides which is which, because only its
// internal keys are ever looked up. Type 0x02 stores an integer, 0x03 a float.
//
// `pendingbits` holds the bits the server has not answered for yet; for an app
// the account does not own nothing is ever confirmed, so the unlock is the
// union of the two masks. An `AchievementTimes` entry is a third source: a
// dated bit counts as unlocked even when both masks are clear.
struct Walker final : kv1::Visitor {
    explicit Walker(UserState& state) : out(state) {}

    UserState& out;
    bool inCache = false;
    std::uint32_t bucket = 0;
    bool haveBucket = false;
    std::uint32_t dataMask = 0;
    std::uint32_t pendingMask = 0;
    bool haveMask = false;
    float dataValue = 0.0f;
    bool haveValue = false;
    bool inTimes = false;
    std::unordered_map<std::uint32_t, std::uint32_t> times; // bit -> unix time

    static bool ParseId(const std::string& text, std::uint32_t& value) {
        const auto [end, error] = std::from_chars(text.data(), text.data() + text.size(), value);
        return error == std::errc{} && end == text.data() + text.size();
    }

    static float FromBits(std::uint32_t bits) {
        float value = 0.0f;
        std::memcpy(&value, &bits, sizeof(value));
        return value;
    }

    // Publishes the bucket collected so far. Called on the closing 0x08 and once
    // more at end of stream, so a truncated final bucket still counts.
    void FlushBucket() {
        if (haveBucket) {
            if (haveMask) {
                const std::uint32_t mask = dataMask | pendingMask;
                for (std::uint32_t bit = 0; bit < 32; ++bit) {
                    if ((mask & (1u << bit)) == 0)
                        continue;
                    const auto it = times.find(bit);
                    out.unlocked[bucket * 256u + bit] = it == times.end() ? 0u : it->second;
                }
            }
            for (const auto& [bit, time] : times) {
                const std::uint32_t key = bucket * 256u + bit;
                if (!out.unlocked.contains(key))
                    out.unlocked[key] = time;
            }
            if (haveValue)
                out.values[bucket] = dataValue;
        }
        bucket = 0;
        haveBucket = false;
        dataMask = 0;
        pendingMask = 0;
        haveMask = false;
        dataValue = 0.0f;
        haveValue = false;
        inTimes = false;
        times.clear();
    }

    void OnDictBegin(const std::string& name, int depth) override {
        if (depth == 0) {
            inCache = name == "cache";
            return;
        }
        if (!inCache)
            return;
        if (depth == 1) {
            std::uint32_t id = 0;
            haveBucket = ParseId(name, id);
            bucket = id;
        } else if (depth == 2 && name == "AchievementTimes") {
            inTimes = true;
        }
    }

    void OnDictEnd(int depth) override {
        if (depth == 1)
            FlushBucket();
        else if (depth == 2)
            inTimes = false;
        else if (depth == 0)
            inCache = false;
    }

    void OnInt32(const std::string& key, std::uint8_t type, std::int32_t value, int depth) override {
        if (!inCache)
            return;
        if (depth == 2 && key == "data") {
            dataMask = static_cast<std::uint32_t>(value);
            haveMask = true;
            dataValue = type == kTypeFloat ? FromBits(static_cast<std::uint32_t>(value)) : static_cast<float>(value);
            haveValue = true;
        } else if (depth == 2 && key == "pendingbits") {
            pendingMask = static_cast<std::uint32_t>(value);
            haveMask = true;
        } else if (depth == 3 && inTimes) {
            std::uint32_t bit = 0;
            if (ParseId(key, bit) && bit < 32 && value > 0)
                times[bit] = static_cast<std::uint32_t>(value);
        }
    }
};

} // namespace

bool ParseUserState(const std::uint8_t* data, std::size_t size, UserState& out) {
    if (!data || size == 0)
        return false;
    Walker walker(out);
    const bool wellFormed = kv1::WalkBinary(data, size, walker);
    walker.FlushBucket();
    return wellFormed;
}

} // namespace ac::achieve
