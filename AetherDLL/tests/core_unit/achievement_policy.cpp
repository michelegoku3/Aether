#include "cases.h"

#include <cstdint>
#include <string>

#include "hooks/wire/AchievementUiPolicy.h"

namespace {

ac::achieve::SchemaEntry Entry(std::uint32_t key, std::uint32_t groupid, const char* name) {
    ac::achieve::SchemaEntry entry;
    entry.hasKey = true;
    entry.key = key;
    entry.groupid = groupid;
    entry.name = name;
    return entry;
}

} // namespace

void AchievementPolicy() {
    using namespace ac::achieve;

    Input base;
    base.managed = true;
    base.schemaKnown = true;
    base.stateKnown = true;
    base.version = 9;
    base.hash = 2268056439u;
    base.entries = {Entry(256, 0, "FIRST_TOOL"), Entry(258, 0, "ALL_TOOLS"), Entry(259, 0, "FIRST_SILK_SKILL")};
    base.unlocked = {{258, 1700000000u}};

    // --- guard order: every "pass" leaves Valve's frame alone -----------------
    {
        Input in = base;
        in.managed = false;
        auto d = Decide(in);
        CHECK(d.verdict == Verdict::Passthrough);
        CHECK(std::string(d.reason) == "not-managed");

        in = base;
        in.familyShared = true;
        d = Decide(in);
        CHECK(d.verdict == Verdict::Passthrough);
        CHECK(std::string(d.reason) == "family-shared");

        in = base;
        in.schemaKnown = false;
        d = Decide(in);
        CHECK(d.verdict == Verdict::Passthrough);
        CHECK(std::string(d.reason) == "no-schema");

        in = base;
        in.hash = 0;
        d = Decide(in);
        CHECK(d.verdict == Verdict::Passthrough);
        CHECK(std::string(d.reason) == "no-hash");

        in = base;
        in.stateKnown = false;
        d = Decide(in);
        CHECK(d.verdict == Verdict::Passthrough);
        CHECK(std::string(d.reason) == "no-state");

        in = base;
        in.unlocked.clear();
        d = Decide(in);
        CHECK(d.verdict == Verdict::Passthrough);
        CHECK(std::string(d.reason) == "nothing-unlocked");
    }

    // --- partial progress: one of three unlocked, group not complete ----------
    {
        const auto d = Decide(base);
        CHECK(d.verdict == Verdict::Rebuild);
        CHECK(d.reply.entries.size() == 1);
        CHECK(d.reply.entries[0].key == 258);
        CHECK(d.reply.entries[0].unlockTime == 1700000000u);
        CHECK(d.reply.version == 9);
        CHECK(d.reply.hash == base.hash);
        CHECK(d.reply.groups.size() == 1);
        CHECK(d.reply.groups[0].groupid == 0);
        CHECK(d.reply.groups[0].total == 3);
        CHECK(d.reply.groups[0].unlocked == 1);
        CHECK(d.reply.groups[0].completedAt == 0); // not complete
    }

    // --- complete group: completion time kept ---------------------------------
    {
        Input in = base;
        in.entries = {Entry(256, 0, "ONLY")};
        in.unlocked = {{256, 1759087575u}};
        const auto d = Decide(in);
        CHECK(d.verdict == Verdict::Rebuild);
        CHECK(d.reply.groups.size() == 1);
        CHECK(d.reply.groups[0].total == 1);
        CHECK(d.reply.groups[0].unlocked == 1);
        CHECK(d.reply.groups[0].completedAt == 1759087575u);
    }

    // --- two groups: only the one with progress is emitted --------------------
    {
        Input in = base;
        in.entries = {Entry(256, 0, "A"), Entry(257, 0, "B"), Entry(768, 3, "C"), Entry(769, 3, "D")};
        in.unlocked = {{256, 100u}, {257, 200u}};
        const auto d = Decide(in);
        CHECK(d.verdict == Verdict::Rebuild);
        CHECK(d.reply.groups.size() == 1);
        CHECK(d.reply.groups[0].groupid == 0);
        CHECK(d.reply.groups[0].total == 2);
        CHECK(d.reply.groups[0].unlocked == 2);
        CHECK(d.reply.groups[0].completedAt == 200u);
    }

    // --- order is by internal_key, not input order ----------------------------
    {
        Input in = base;
        in.entries = {Entry(768, 3, "C"), Entry(256, 0, "A"), Entry(512, 1, "B")};
        in.unlocked = {{768, 30u}, {256, 10u}, {512, 20u}};
        const auto d = Decide(in);
        CHECK(d.reply.entries.size() == 3);
        CHECK(d.reply.entries[0].key == 256);
        CHECK(d.reply.entries[1].key == 512);
        CHECK(d.reply.entries[2].key == 768);
        CHECK(d.reply.groups.size() == 3);
        CHECK(d.reply.groups[0].groupid == 0);
        CHECK(d.reply.groups[1].groupid == 1);
        CHECK(d.reply.groups[2].groupid == 3);
    }

    // --- an unlock with no schema entry is never emitted ----------------------
    {
        Input in = base;
        in.entries = {Entry(256, 0, "A")};
        in.unlocked = {{999, 5u}, {256, 7u}};
        const auto d = Decide(in);
        CHECK(d.reply.entries.size() == 1);
        CHECK(d.reply.entries[0].key == 256);
    }

    // --- internal_key 0 is a real key: bucket 0, bit 0 ------------------------
    {
        Input in = base;
        in.entries = {Entry(0, 0, "FIRST"), Entry(1, 0, "SECOND")};
        in.unlocked = {{0, 1234u}};
        const auto d = Decide(in);
        CHECK(d.verdict == Verdict::Rebuild);
        CHECK(d.reply.entries.size() == 1);
        CHECK(d.reply.entries[0].key == 0);
        CHECK(d.reply.entries[0].unlockTime == 1234u);
        CHECK(d.reply.groups[0].total == 2);
        CHECK(d.reply.groups[0].unlocked == 1);
    }

    // --- a schema entry without internal_key is not keyed and not counted ----
    {
        Input in = base;
        SchemaEntry unkeyed;
        unkeyed.hasKey = false;
        unkeyed.groupid = 0;
        unkeyed.name = "NO_KEY";
        in.entries = {Entry(256, 0, "A"), unkeyed};
        in.unlocked = {{256, 5u}};
        const auto d = Decide(in);
        CHECK(d.verdict == Verdict::Rebuild);
        CHECK(d.reply.groups[0].total == 1); // the unkeyed entry is invisible
        CHECK(d.reply.groups[0].unlocked == 1);
        CHECK(d.reply.groups[0].completedAt == 5u);
    }
}
