#include "cases.h"

#include "hooks/aetheronline/PayloadAttest.h"

// The version pin is a release identity, not an ordering: any difference in
// either version word marks a foreign payload and must refuse it.
void PayloadAttestTest() {
    using ac::hooks::payloadattest::VersionsMatch;

    CHECK(VersionsMatch(0x00010002u, 0x00030004u, 0x00010002u, 0x00030004u));
    CHECK(!VersionsMatch(0x00010002u, 0x00030004u, 0x00010003u, 0x00030004u));
    CHECK(!VersionsMatch(0x00010002u, 0x00030004u, 0x00010002u, 0x00030005u));
    // A missing version resource (zeroed words) never matches a real build.
    CHECK(!VersionsMatch(0, 0, 0x00010002u, 0x00030004u));
    CHECK(!VersionsMatch(0x00010002u, 0x00030004u, 0, 0));
}
