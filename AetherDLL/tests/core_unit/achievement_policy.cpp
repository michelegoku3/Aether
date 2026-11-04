#include "cases.h"

#include <cstdint>
#include <string>

#include "hooks/wire/AchievementUiPolicy.h"

namespace {

ac::achieve::SchemaEntry Entry(std::uint32_t key, std::uint32_t groupid) {
    ac::achieve::SchemaEntry entry;
    entry.key = key;
    entry.groupid = groupid;
    return entry;
}

} // namespace

void AchievementPolicy() {
    using namespace ac::achieve;

    Input base;
    base.managed = true;
    base.schemaKnown = true;
    base.stateLoaded = true;
    base.entries = {Entry(256, 0), Entry(258, 0), Entry(259, 0)};
    base.groups = {0};
    base.unlocked = {{258, 1700000000u}};

    // --- guard order ---------------------------------------------------------
    {
        Input in = base;
        in.managed = false;
        auto d = Decide(in);
        CHECK(d.verdict == Verdict::Passthrough);
        CHECK(std::string(d.reason) == "not-managed");

        // No schema: the caller holds the reply until the schema arrives.
        in = base;
        in.schemaKnown = false;
        in.entries.clear();
        d = Decide(in);
        CHECK(d.verdict == Verdict::Hold);
        CHECK(std::string(d.reason) == "no-schema");

        // A captured schema with no achievements is the same cold case.
        in = base;
        in.entries.clear();
        d = Decide(in);
        CHECK(d.verdict == Verdict::Hold);
        CHECK(std::string(d.reason) == "no-schema");

        // The local .bin read has not finished yet.
        in = base;
        in.stateLoaded = false;
        d = Decide(in);
        CHECK(d.verdict == Verdict::Hold);
        CHECK(std::string(d.reason) == "no-state");
    }

    // --- partial progress: every schema achievement is answered --------------
    {
        const auto d = Decide(base);
        CHECK(d.verdict == Verdict::Rebuild);
        CHECK(std::string(d.reason) == "rebuilt");
        CHECK(d.reply.entries.size() == 3);
        CHECK(d.reply.entries[0].key == 256);
        CHECK(!d.reply.entries[0].unlocked);
        CHECK(d.reply.entries[1].key == 258);
        CHECK(d.reply.entries[1].unlocked);
        CHECK(d.reply.entries[1].unlockTime == 1700000000u);
        CHECK(d.reply.entries[2].key == 259);
        CHECK(!d.reply.entries[2].unlocked);
        CHECK(d.reply.groups.size() == 1);
        CHECK(d.reply.groups[0].groupid == 0);
    }

    // --- no unlocks at all: still one entry per achievement ------------------
    {
        Input in = base;
        in.unlocked.clear();
        const auto d = Decide(in);
        CHECK(d.verdict == Verdict::Rebuild);
        CHECK(d.reply.entries.size() == 3);
        CHECK(!d.reply.entries[0].unlocked);
        CHECK(!d.reply.entries[1].unlocked);
        CHECK(!d.reply.entries[2].unlocked);
        CHECK(d.reply.groups.size() == 1);
        CHECK(d.reply.groups[0].groupid == 0);
    }

    // --- order is the schema order, not the bit order nor the map order ------
    {
        Input in = base;
        in.entries = {Entry(768, 3), Entry(256, 0), Entry(512, 1)};
        in.groups = {3, 0, 1};
        in.unlocked = {{768, 30u}, {256, 10u}, {512, 20u}};
        const auto d = Decide(in);
        CHECK(d.reply.entries.size() == 3);
        CHECK(d.reply.entries[0].key == 768);
        CHECK(d.reply.entries[1].key == 256);
        CHECK(d.reply.entries[2].key == 512);
        CHECK(d.reply.groups.size() == 3);
        CHECK(d.reply.groups[0].groupid == 3);
        CHECK(d.reply.groups[1].groupid == 0);
        CHECK(d.reply.groups[2].groupid == 1);
    }

    // --- internal_key 0 is a real key: bucket 0, bit 0 ------------------------
    {
        Input in = base;
        in.entries = {Entry(0, 0), Entry(1, 0)};
        in.groups = {0};
        in.unlocked = {{0, 1234u}};
        const auto d = Decide(in);
        CHECK(d.verdict == Verdict::Rebuild);
        CHECK(d.reply.entries.size() == 2);
        CHECK(d.reply.entries[0].key == 0);
        CHECK(d.reply.entries[0].unlocked);
        CHECK(d.reply.entries[0].unlockTime == 1234u);
        CHECK(!d.reply.entries[1].unlocked);
    }

    // --- an unlock with no schema entry is never emitted ----------------------
    {
        Input in = base;
        in.entries = {Entry(256, 0)};
        in.unlocked = {{999, 5u}, {256, 7u}};
        const auto d = Decide(in);
        CHECK(d.reply.entries.size() == 1);
        CHECK(d.reply.entries[0].key == 256);
        CHECK(d.reply.entries[0].unlocked);
    }

    // --- a schema without groups emits none (the client reads its own) -------
    {
        Input in = base;
        in.groups.clear();
        const auto d = Decide(in);
        CHECK(d.verdict == Verdict::Rebuild);
        CHECK(d.reply.groups.empty());
    }
}
