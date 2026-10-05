#pragma once
#include <charconv>
#include <cstdint>
#include <string>
#include <unordered_set>
#include "utils/KeyValues.h"

// Pure visitor shared by the disk adapter and synthetic KV1 tests.
namespace ac::backup::statscache::detail {
    // Estrae i bucket achievement (chiavi della sezione "stats" che contengono
    // una sotto-sezione "bits"). Formato schema: <appid> { stats { <bucket> {
    // bits { ... } } } }: si raccogliono le chiavi dei dizionari a profondità 2
    // che contengono "bits".
    // Bucket achievement via walker KV1 condiviso (utils/KeyValues, P11):
    // sezione "stats" a profondità 1; un bucket (profondità 2) è achievement
    // solo se il suo dizionario "bits" ha almeno una voce.
    struct SchemaBucketWalker final : kv1::Visitor {
        std::unordered_set<std::uint32_t> buckets;
        bool inStats = false;
        int bucketDepth = -1;          // profondità dell'entry del bucket corrente
        std::string currentBucket;
        bool inBits = false;
        int bitsEntries = 0;

        void FinalizeBucket() {
            if (inStats && bitsEntries > 0) {
                std::uint32_t id = 0;
                const auto [end, error] = std::from_chars(
                    currentBucket.data(), currentBucket.data() + currentBucket.size(), id);
                if (error == std::errc{} && end == currentBucket.data() + currentBucket.size())
                    buckets.insert(id);
            }
            inBits = false;
            bucketDepth = -1;
            bitsEntries = 0;
        }
        void OnDictBegin(const std::string& name, int depth) override {
            if (depth == 1 && name == "stats") inStats = true;
            if (inStats && depth == 2) {
                currentBucket = name;   // potenziale bucket
                bucketDepth = 2;
                bitsEntries = 0;
            }
            // Semantica identica al parser storico: contano solo le voci
            // DIZIONARIO dentro "bits" (profondità 4).
            if (bucketDepth == 2 && depth == 3 && name == "bits") inBits = true;
            if (inBits && depth == 4) ++bitsEntries;
        }
        void OnDictEnd(int depth) override {
            if (depth == 3) inBits = false;
            if (bucketDepth == 2 && depth == 2) FinalizeBucket();
            if (inStats && depth == 1) inStats = false;   // chiusa la sezione "stats"
        }
    };

}  // namespace ac::backup::statscache::detail
