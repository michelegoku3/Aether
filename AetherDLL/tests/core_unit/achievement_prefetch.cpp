#include "cases.h"

#include <cstdint>
#include <string>

#include "hooks/wire/AchievementPrefetchPolicy.h"

void AchievementPrefetchPolicyTest() {
    namespace pf = ac::hooks::wire::prefetch;

    // The marker file is a hint, never a contract: "<appid> <unix>" lines, one
    // per app whose reply listed no achievement. Malformed lines are dropped and
    // a later line for the same app wins, so a hand-edited file stays usable.
    const std::string text = "4809710 1794804374\n"
                             "105600 1794800000\r\n" // CRLF from a Windows editor
                             "bad line\n"
                             "0 1794800000\n"       // app 0 does not exist
                             "4809710 1794900000\n" // later line wins
                             "105600\n"
                             "3618390 not-a-number\n"
                             "  \n";
    const pf::Markers markers = pf::ParseMarkers(text);
    CHECK_EQ(markers.size(), static_cast<std::size_t>(2));
    CHECK_EQ(markers.at(4809710u), 1794900000ull);
    CHECK_EQ(markers.at(105600u), 1794800000ull);

    // Sorted output: a stable, diffable file for a support bundle.
    CHECK_EQ(pf::EncodeMarkers(markers), std::string("105600 1794800000\n4809710 1794900000\n"));
    CHECK_EQ(pf::ParseMarkers(pf::EncodeMarkers(markers)), markers);
    CHECK(pf::ParseMarkers("").empty());
    CHECK(pf::EncodeMarkers({}).empty());

    // An empty answer is trusted for a week: within that window Aether does not
    // ask again, after it it does (an update can add achievements).
    constexpr std::uint64_t now = 1795000000ull;
    CHECK(pf::IsAnswerFresh(now - 1, now));
    CHECK(pf::IsAnswerFresh(now - (pf::kEmptyAnswerTtlSeconds - 1), now));
    CHECK(!pf::IsAnswerFresh(now - pf::kEmptyAnswerTtlSeconds, now));
    CHECK(!pf::IsAnswerFresh(now - (pf::kEmptyAnswerTtlSeconds + 1), now));
    CHECK(!pf::IsAnswerFresh(0, now));        // no answer recorded
    CHECK(!pf::IsAnswerFresh(now + 60, now)); // clock moved backwards: ask

    // The decision: ask only for what has no schema file and no fresh empty
    // answer. A schema file on disk is proof the catalog was obtained (Steam's
    // download or an earlier transcription).
    CHECK(pf::ShouldAsk(false, false));
    CHECK(!pf::ShouldAsk(true, false));
    CHECK(!pf::ShouldAsk(false, true));
    CHECK(!pf::ShouldAsk(true, true));
}
