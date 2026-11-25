#include "cases.h"

#include "utils/PatternSidecar.h"
#include "utils/PatternState.h"
#include "utils/PatternStateParse.h"
#include "utils/WinFile.h"

#include <cstdint>
#include <string>

using ac::pattern::AdoptDownload;
using ac::pattern::Lookup;
using ac::pattern::ParseState;
using ac::pattern::PlanRefresh;

namespace {

std::string ShaOf(char nibble) { return std::string(64, nibble); }

// PlanRefresh is the whole cache decision in one pure function, so the matrix
// below is the contract PatternCache relies on.
void CheckRefreshMatrix() {
    // No usable cache: the caller downloads as it always did; the plan has no
    // say there.
    CHECK(!PlanRefresh(false, 0, 0, true).needed);
    CHECK(!PlanRefresh(false, 5, 0, false).needed);

    // A newer generation always refreshes: the bump is the author's explicit
    // statement that the published table changed.
    const auto versioned = PlanRefresh(true, 2, 1, true);
    CHECK(versioned.needed);
    CHECK(versioned.versioned);
    const auto firstSighting = PlanRefresh(true, 1, 0, false);
    CHECK(firstSighting.needed);
    CHECK(firstSighting.versioned);
    const auto preferredFirstSighting = PlanRefresh(true, 1, 0, true);
    CHECK(preferredFirstSighting.needed);
    CHECK(preferredFirstSighting.versioned);

    // Same generation: only the provenance probe (a better source exists).
    CHECK(!PlanRefresh(true, 3, 3, true).needed);
    const auto probe = PlanRefresh(true, 3, 3, false);
    CHECK(probe.needed);
    CHECK(!probe.versioned);

    // Index unavailable (generation 0) on a table applied at 0: probe only.
    CHECK(!PlanRefresh(true, 0, 0, true).needed);
    CHECK(PlanRefresh(true, 0, 0, false).needed);

    // A remote generation behind the applied one never moves backwards.
    CHECK(!PlanRefresh(true, 1, 2, true).needed);

    // A newer generation whose download is byte-identical to the cache is a
    // stale fetch, not a change: adopt nothing, keep the stored generation, and
    // let the next launch ask again. A cache that records no generation (a
    // legacy sidecar) adopts the bytes it already holds, so it converges on the
    // first launch.
    CHECK(!AdoptDownload(true, 1, true));
    CHECK(AdoptDownload(true, 0, true));
    CHECK(AdoptDownload(true, 1, false));
    CHECK(AdoptDownload(false, 1, true));
}

} // namespace

void PatternStateParseTest() {
    CheckRefreshMatrix();

    const std::string shaA = ShaOf('a');
    const std::string shaB = ShaOf('b');

    // The published shape: header comment, blank lines, one section per module.
    const std::string body = "# Aether patterns schema versioning.\n"
                             "\n"
                             "[steamclient]\n" +
                             shaA + " 1\n" + shaB + " 7\n" + "\n" + "[steamclientipc]\n" + shaA + " 3\n";
    const auto parsed = ParseState(body);
    CHECK_EQ(parsed.rejected, 0u);
    CHECK_EQ(parsed.modules.size(), 2u);
    CHECK_EQ(Lookup(parsed.modules, "steamclient", shaA), 1u);
    CHECK_EQ(Lookup(parsed.modules, "steamclient", shaB), 7u);
    CHECK_EQ(Lookup(parsed.modules, "steamclientipc", shaA), 3u);
    // Unknown module or SHA: an entry the consumer has no opinion about.
    CHECK_EQ(Lookup(parsed.modules, "steamui", shaA), 0u);
    CHECK_EQ(Lookup(parsed.modules, "steamclient", ShaOf('c')), 0u);

    // A UTF-8 BOM and CRLF line endings are what a Windows checkout produces.
    const std::string bomCrlf = std::string("\xEF\xBB\xBF") + "# comment\r\n[steamclient]\r\n" + shaA + " 2\r\n";
    const auto windows = ParseState(bomCrlf);
    CHECK_EQ(windows.rejected, 0u);
    CHECK_EQ(Lookup(windows.modules, "steamclient", shaA), 2u);

    // Generations are not bounded by 32 bits.
    const auto wide = ParseState("[steamclient]\n" + shaA + " 4294967296\n");
    CHECK_EQ(Lookup(wide.modules, "steamclient", shaA), 4294967296ull);

    // Every malformed line is counted and skipped; a broken header clears the
    // section so following entries are not attributed to the previous module.
    const std::string malformed = "[steamclient]\n"
                                  "short 1\n" +
                                  ShaOf('g') + " 1\n" + shaA + " +5\n" + shaB + " 1_0\n" + ShaOf('d') + " 0\n" +
                                  ShaOf('e') + "\n" + ShaOf('f') + " 4 extra\n" + "   \n" + "[broken\n" + ShaOf('1') +
                                  " 9\n";
    const auto broken = ParseState(malformed);
    CHECK_EQ(broken.rejected, 9u);
    CHECK(broken.modules.empty());

    // A duplicate keeps the larger generation instead of resetting it.
    const auto duplicate = ParseState("[steamclient]\n" + shaA + " 2\n" + shaA + " 5\n");
    CHECK_EQ(duplicate.rejected, 1u);
    CHECK_EQ(Lookup(duplicate.modules, "steamclient", shaA), 5u);

    // Case is normalized on both sides: the release writes lowercase, and a
    // hand-edited file must not create a second key.
    std::string upperSha = shaA;
    for (char& c : upperSha)
        if (c >= 'a' && c <= 'f')
            c = static_cast<char>(c - 'a' + 'A');
    const auto cased = ParseState("[SteamClient]\n" + upperSha + " 4\n");
    CHECK_EQ(Lookup(cased.modules, "steamclient", shaA), 4u);
    CHECK_EQ(cased.modules.size(), 1u);

    // Nothing to read is not an error.
    const auto empty = ParseState("");
    CHECK(empty.modules.empty());
    CHECK_EQ(empty.rejected, 0u);
}

void PatternSidecarTest() {
    Fixture f;

    // A missing sidecar is an unknown origin at generation 0, which is exactly
    // how every cache written before the index existed reads.
    const auto missing = ac::pattern::sidecar::Read((f.dir / "missing.toml.src").string());
    CHECK(missing.source.empty());
    CHECK_EQ(missing.version, 0u);

    // One-line sidecars from older builds keep their source and read as 0.
    const auto legacy = f.Write("legacy.toml.src", "migo:raw");
    const auto legacyRead = ac::pattern::sidecar::Read(legacy.string());
    CHECK_EQ(legacyRead.source, std::string("migo:raw"));
    CHECK_EQ(legacyRead.version, 0u);

    const auto twoLines = f.Write("two.toml.src", "migo:cdn\nversion=7\n");
    const auto twoRead = ac::pattern::sidecar::Read(twoLines.string());
    CHECK_EQ(twoRead.source, std::string("migo:cdn"));
    CHECK_EQ(twoRead.version, 7u);

    const auto crlf = f.Write("crlf.toml.src", "migo:raw\r\nversion=3\r\n");
    const auto crlfRead = ac::pattern::sidecar::Read(crlf.string());
    CHECK_EQ(crlfRead.source, std::string("migo:raw"));
    CHECK_EQ(crlfRead.version, 3u);

    // A version line we cannot trust reads as 0, so the next boot asks the
    // index again instead of believing a corrupt number.
    const auto bad = f.Write("bad.toml.src", "migo:raw\nversion=+3\n");
    const auto badRead = ac::pattern::sidecar::Read(bad.string());
    CHECK_EQ(badRead.source, std::string("migo:raw"));
    CHECK_EQ(badRead.version, 0u);

    const auto written = (f.dir / "written.toml.src").string();
    CHECK(ac::pattern::sidecar::Write(written, "migo:raw", 12));
    const auto writtenRead = ac::pattern::sidecar::Read(written);
    CHECK_EQ(writtenRead.source, std::string("migo:raw"));
    CHECK_EQ(writtenRead.version, 12u);

    // An empty source still round-trips: the version line is what matters.
    CHECK(ac::pattern::sidecar::Write(written, "", 4));
    const auto emptySource = ac::pattern::sidecar::Read(written);
    CHECK(emptySource.source.empty());
    CHECK_EQ(emptySource.version, 4u);

    // The refresh rule compares the candidate with the cache byte for byte.
    const auto left = f.Write("left.toml", "a = 1\n");
    const auto right = f.Write("right.toml", "a = 1\n");
    const auto other = f.Write("other.toml", "a = 2\n");
    CHECK(ac::winfile::FilesEqual(left.string(), right.string()));
    CHECK(!ac::winfile::FilesEqual(left.string(), other.string()));
    CHECK(!ac::winfile::FilesEqual(left.string(), (f.dir / "absent.toml").string()));
}
