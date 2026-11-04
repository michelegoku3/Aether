#pragma once

#include <algorithm>
#include <cstdint>
#include <string>
#include <unordered_map>
#include <vector>

// ---------------------------------------------------------------------------
// Pure decision + body model for the client achievement page.
//
// The client asks for the schema (Player.GetGameAchievements#1) and the user
// state (Player.GetUserAchievements#1) together, before either reply exists.
// For a managed app the server answers the state with an empty body, so the
// page shows nothing even though the local .bin holds the real unlocks.
//
// This header decides whether to answer from local state and builds the body
// model. It is free of logs, g_state, protobuf and Windows so the decision
// table can be tested directly.
//
// Reply shape measured on apps the account really owns:
// only unlocked achievements appear, in schema order; the group block carries
// the unlocked count, a complete flag and the completion time.
// ---------------------------------------------------------------------------
namespace ac::achieve {

struct SchemaEntry {
    std::uint32_t key = 0; // internal_key = bucket * 256 + bit
    std::uint32_t groupid = 0;
    std::string name; // internal_name (diagnostics only)
};

struct ReplyEntry {
    std::uint32_t key = 0;
    std::uint32_t unlockTime = 0;
};

struct ReplyGroup {
    std::uint32_t groupid = 0;
    std::uint32_t total = 0;
    std::uint32_t unlocked = 0;
    std::uint32_t completedAt = 0; // 0 unless the group is complete
};

struct Reply {
    std::int32_t version = 0;
    std::uint32_t hash = 0;
    std::vector<ReplyEntry> entries;
    std::vector<ReplyGroup> groups;
};

enum class Verdict { Passthrough, Rebuild };

struct Input {
    bool managed = false;      // configured .lua app, not genuinely owned
    bool familyShared = false; // lent by a family member: no honest local state
    bool schemaKnown = false;  // a GetGameAchievements reply was captured
    bool stateKnown = false;   // the user .bin exists and parsed
    std::int32_t version = 0;
    std::uint32_t hash = 0;
    std::vector<SchemaEntry> entries;
    std::unordered_map<std::uint32_t, std::uint32_t> unlocked; // key -> time
};

struct Decision {
    Verdict verdict = Verdict::Passthrough;
    const char* reason = "";
    Reply reply;
};

// Decision table, in the order the guards apply. Any "pass" leaves Valve's
// frame untouched: a page that shows less is better than one that lies.
inline Decision Decide(const Input& in) {
    Decision decision;
    if (!in.managed) {
        decision.reason = "not-managed";
        return decision;
    }
    if (in.familyShared) {
        decision.reason = "family-shared";
        return decision;
    }
    if (!in.schemaKnown) {
        decision.reason = "no-schema";
        return decision;
    }
    if (in.hash == 0) {
        decision.reason = "no-hash";
        return decision;
    }
    if (!in.stateKnown) {
        decision.reason = "no-state";
        return decision;
    }

    // Deterministic order regardless of the map iteration order: the client
    // compares the reply against the schema it holds.
    std::vector<const SchemaEntry*> ordered;
    ordered.reserve(in.entries.size());
    for (const SchemaEntry& entry : in.entries) {
        if (entry.key != 0)
            ordered.push_back(&entry);
    }
    std::sort(ordered.begin(), ordered.end(),
              [](const SchemaEntry* a, const SchemaEntry* b) { return a->key < b->key; });

    std::unordered_map<std::uint32_t, ReplyGroup> groups;
    for (const SchemaEntry* entry : ordered) {
        ReplyGroup& group = groups[entry->groupid];
        group.groupid = entry->groupid;
        ++group.total;

        const auto it = in.unlocked.find(entry->key);
        if (it == in.unlocked.end())
            continue;
        decision.reply.entries.push_back(ReplyEntry{entry->key, it->second});
        ++group.unlocked;
        group.completedAt = std::max(group.completedAt, it->second);
    }

    if (decision.reply.entries.empty()) {
        // Nothing unlocked: Valve's own reply is exactly the empty version+hash
        // body, so there is nothing to add.
        decision.reason = "nothing-unlocked";
        return decision;
    }

    for (auto& [groupid, group] : groups) {
        if (group.unlocked == 0)
            continue;
        if (group.unlocked != group.total)
            group.completedAt = 0;
        decision.reply.groups.push_back(group);
    }
    std::sort(decision.reply.groups.begin(), decision.reply.groups.end(),
              [](const ReplyGroup& a, const ReplyGroup& b) { return a.groupid < b.groupid; });

    decision.reply.version = in.version;
    decision.reply.hash = in.hash;
    decision.verdict = Verdict::Rebuild;
    decision.reason = "rebuilt";
    return decision;
}

} // namespace ac::achieve
