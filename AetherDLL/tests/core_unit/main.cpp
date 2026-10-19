#include "cases.h"

#include <iostream>
#include <utility>

// Single dispatch table for all core unit suites: ctest runs one case per
// process (isolation + per-case timeouts), while no argument runs everything
// in one process for local iteration.
int main(int argc, char** argv) {
    static constexpr std::pair<const char*, void (*)()> kCases[] = {
        {"strings", Strings},
        {"paths", Paths},
        {"policy", Policy},
        {"worker", Worker},
        {"budget", Budget},
        {"settings", Settings},
        {"registry", Registry},
        {"workers", Workers},
        {"schema_buckets", SchemaBuckets},
        {"workers_reap_slow", WorkersReapSlow},
        {"workers_race", WorkersShutdownRace},
        {"netpacket", NetPacket},
        {"sentinel", Sentinel},
        {"structguard", StructGuard},
        {"gamesplayed_format", GamesPlayedFormatTest},
        {"donorpool", DonorPoolTest},
        {"keyvalues", KeyValuesTest},
        {"vdftext", VdfTextTest},
        {"jsonutil_escapes", JsonEscapesTest},
        {"manifest_identity", ManifestIdentityTest},
        {"ipcreply", IpcReplyTest},
        {"snapshot_merge", SnapshotMergeTest},
        {"ttlcache", TtlCacheTest},
        {"hexcodec", HexCodecTest},
        {"signature", SignatureTest},
        {"ipcspec", IpcSpecParseTest},
        {"jsonwriter", JsonWriterTest},
        {"manifest_contract", ManifestContractTest},
        {"acfowner", AcfOwnerTest},
        {"steamuicss", SteamUiCssTest},
        {"steamuipatch", SteamUiPatchTest},
    };
    try {
        if (argc == 1) {
            for (const auto& [name, fn] : kCases) {
                fn();
                std::cout << name << ": PASS\n";
            }
            return 0;
        }
        CHECK(argc == 2);
        const std::string test = argv[1];
        for (const auto& [name, fn] : kCases) {
            if (test == name) {
                fn();
                std::cout << test << ": PASS\n";
                return 0;
            }
        }
        CHECK(false);
        return 1;
    } catch (const std::exception& e) {
        std::cerr << e.what() << '\n';
        return 1;
    }
}
