#include "pch.h"
#include "hooks/wire/AchievementStatsSchema.h"

#include <charconv>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

#include "utils/KeyValues.h"

namespace ac::achieve {
namespace {

// Walks the schema once. The "stats" section sits under the appid section in
// the schemas measured on the beta client, but a schema that puts it at the top
// level parses too: every depth below is relative to where "stats" was found.
//
// Achievement bucket names and the stat names an operands points at live in the
// same section and either can come first, so operands are resolved after the
// walk instead of on the way down.
struct Walker final : kv1::Visitor {
    explicit Walker(StatsSchema& schema) : out(schema) {}

    StatsSchema& out;

    bool inStats = false;
    int base = -1; // depth of the "stats" dictionary
    bool inBits = false;
    std::uint32_t bucket = 0;
    bool haveBucket = false;
    bool inProgress = false;
    bool inValue = false;
    std::uint32_t bit = 0;
    bool haveBit = false;
    std::string operation;
    std::string operand1;

    std::unordered_map<std::string, std::uint32_t> idByName;

    // One achievement with a "statvalue" counter, before its stat name is
    // resolved to a stat id.
    struct Pending {
        std::uint32_t key = 0;
        std::string statName;
    };
    std::vector<Pending> pending;

    static bool ParseId(const std::string& text, std::uint32_t& value) {
        const auto [end, error] = std::from_chars(text.data(), text.data() + text.size(), value);
        return error == std::errc{} && end == text.data() + text.size();
    }

    void OnDictBegin(const std::string& name, int depth) override {
        if (!inStats) {
            if (name == "stats" && depth <= 1) {
                inStats = true;
                base = depth;
            }
            return;
        }
        if (depth == base + 1) {
            haveBucket = ParseId(name, bucket);
            haveBit = false;
            return;
        }
        if (depth == base + 2) {
            inBits = name == "bits";
            return;
        }
        if (depth == base + 3 && inBits) {
            haveBit = ParseId(name, bit) && bit < 32;
            operation.clear();
            operand1.clear();
            return;
        }
        if (depth == base + 4 && haveBit && name == "progress") {
            inProgress = true;
            operation.clear();
            operand1.clear();
            return;
        }
        if (depth == base + 5 && inProgress && name == "value")
            inValue = true;
    }

    void OnDictEnd(int depth) override {
        if (!inStats)
            return;
        if (depth == base) {
            inStats = false;
            base = -1;
            return;
        }
        if (depth == base + 5) {
            inValue = false;
            return;
        }
        if (depth == base + 4) {
            inProgress = false;
            return;
        }
        if (depth == base + 3) {
            // The bit closed: keep it only when the counter is a stat value.
            if (inBits && haveBit && operation == "statvalue" && !operand1.empty())
                pending.push_back(Pending{bucket * 256u + bit, operand1});
            haveBit = false;
            return;
        }
        if (depth == base + 2) {
            inBits = false;
            return;
        }
        if (depth == base + 1)
            haveBucket = false;
    }

    void OnString(const std::string& key, const std::string& value, int depth) override {
        if (!inStats)
            return;
        if (depth == base + 2 && haveBucket && key == "name") {
            idByName[value] = bucket;
            return;
        }
        if (depth == base + 6 && inValue) {
            if (key == "operation")
                operation = value;
            else if (key == "operand1")
                operand1 = value;
        }
    }

    // Resolves the collected operands. A counter whose stat the schema does not
    // declare stays out: the page then shows 0, never a guessed value.
    void Resolve() {
        for (const Pending& entry : pending) {
            const auto it = idByName.find(entry.statName);
            if (it != idByName.end())
                out.progressStat[entry.key] = it->second;
        }
    }
};

} // namespace

bool ParseStatsSchema(const std::uint8_t* data, std::size_t size, StatsSchema& out) {
    if (!data || size == 0)
        return false;
    Walker walker(out);
    const bool wellFormed = kv1::WalkBinary(data, size, walker);
    walker.Resolve();
    return wellFormed;
}

} // namespace ac::achieve
