#include "cases.h"

#include <cstdint>

#include "hooks/wire/AchievementBitfield.h"

void AchievementBitfield() {
    namespace ab = ac::hooks::wire::bitfield;

    // A stat the schema does not list as an achievement bucket is a counter: it
    // grows from bit 0 upward, so a set low bit is a value, never an unlock.
    CHECK_EQ(ab::NewAchievementBits(16, 0, /*schemaBucket=*/false), 0u);
    CHECK_EQ(ab::NewAchievementBits(255, 0, false), 0u);
    CHECK_EQ(ab::NewAchievementBits(0x100, 0, false), 0x100u);
    // Only the additions count: a bit already set is not news, and a bit that
    // disappeared is not an unlock either.
    CHECK_EQ(ab::NewAchievementBits(0x300, 0x100, false), 0x200u);
    CHECK_EQ(ab::NewAchievementBits(0x100, 0x100, false), 0u);
    CHECK_EQ(ab::NewAchievementBits(0x100, 0x300, false), 0u);

    // An achievement bucket: every bit is an achievement, the low ones too.
    // Paystreak's "Sold" is bucket 0, bit 4 — the case the old 8-bit threshold
    // made silent.
    CHECK_EQ(ab::NewAchievementBits(16, 0, true), 16u);
    CHECK_EQ(ab::NewAchievementBits(0x11, 0x10, true), 1u);
    CHECK_EQ(ab::NewAchievementBits(0x10, 0x10, true), 0u);
    CHECK_EQ(ab::NewAchievementBits(0u, 0x10, true), 0u);
    // Bit 31 must not overflow the shift.
    CHECK_EQ(ab::NewAchievementBits(0x80000000u, 0, true), 0x80000000u);
    CHECK_EQ(ab::NewAchievementBits(0xC0000000u, 0x80000000u, true), 0x40000000u);
    CHECK_EQ(ab::NewAchievementBits(0xFFFFFFFFu, 0xFFFFFFFFu, true), 0u);
    // The same commit is read differently for the two kinds of stat.
    CHECK_EQ(ab::NewAchievementBits(0x10, 0, false), 0u);
    CHECK_EQ(ab::NewAchievementBits(0x10, 0, true), 0x10u);
}
