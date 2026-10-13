#pragma once
#include "utils/KeyValues.h"
#include <charconv>
#include <cstdint>
#include <string>
#include <unordered_set>

// Pure visitor shared by the disk adapter and synthetic KV1 tests.
namespace ac::backup::statscache::detail {
// Extracts achievement buckets (keys of the "stats" section holding a
// "bits" subsection). Schema format: <appid> { stats { <bucket> {
// bits { ... } } } }: collects depth-2 dictionary keys containing "bits".
// Achievement buckets via shared KV1 walker (utils/KeyValues, P11):
// depth-1 "stats" section; a depth-2 bucket is an achievement bucket only
// when its "bits" dictionary holds at least one entry.
struct SchemaBucketWalker final : kv1::Visitor {
    std::unordered_set<std::uint32_t> buckets;
    bool inStats = false;
    int bucketDepth = -1; // depth of the current bucket's entry
    std::string currentBucket;
    bool inBits = false;
    int bitsEntries = 0;

    void FinalizeBucket() {
        if (inStats && bitsEntries > 0) {
            std::uint32_t id = 0;
            const auto [end, error] =
                std::from_chars(currentBucket.data(), currentBucket.data() + currentBucket.size(), id);
            if (error == std::errc{} && end == currentBucket.data() + currentBucket.size())
                buckets.insert(id);
        }
        inBits = false;
        bucketDepth = -1;
        bitsEntries = 0;
    }
    void OnDictBegin(const std::string& name, int depth) override {
        if (depth == 1 && name == "stats")
            inStats = true;
        if (inStats && depth == 2) {
            currentBucket = name; // potenziale bucket
            bucketDepth = 2;
            bitsEntries = 0;
        }
        // Same semantics as the historic parser: only DICTIONARY entries
        // inside "bits" count (depth 4).
        if (bucketDepth == 2 && depth == 3 && name == "bits")
            inBits = true;
        if (inBits && depth == 4)
            ++bitsEntries;
    }
    void OnDictEnd(int depth) override {
        if (depth == 3)
            inBits = false;
        if (bucketDepth == 2 && depth == 2)
            FinalizeBucket();
        if (inStats && depth == 1)
            inStats = false; // "stats" section closed
    }
};

} // namespace ac::backup::statscache::detail
