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
// This header decides whether to answer from local state, builds the body
// model, and writes that model into a reply message (`FillReply`, a template so
// no protobuf type is declared here). It is free of logs, g_state and Windows
// so the decision table can be tested directly.
//
// Reply shape measured on apps the account really owns:
// only unlocked achievements appear, in schema order; the group block carries
// the unlocked count, a complete flag and the completion time.
// ---------------------------------------------------------------------------
namespace ac::achieve {

struct SchemaEntry {
    // presence, not value: bucket 0 bit 0 is a real key (internal_key == 0)
    bool hasKey = false;
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
        // An entry without internal_key cannot be keyed by the page, so it
        // neither unlocks nor counts towards its group.
        if (entry.hasKey)
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

// Writes the decision's model into a response message: the two blocks Aether
// owns, and nothing else, so every other field Valve sent is re-emitted. The
// message type is a template parameter to keep this header protobuf-free; the
// wire glue and the golden test both instantiate it with the real response.
template <typename Response> void FillReply(Response& reply, const Decision& decision) {
    reply.clear_achievements();
    reply.clear_groups();
    // The client matches the body against the schema it holds by this pair, so
    // a fallback taken from the captured schema has to be written, not only
    // decided on.
    if (decision.reply.version != 0)
        reply.set_schema_version(decision.reply.version);
    reply.set_schema_hash(decision.reply.hash);
    for (const ReplyEntry& entry : decision.reply.entries) {
        auto* item = reply.add_achievements();
        item->set_internal_key(entry.key);
        item->set_unlocked(true);
        item->set_unlock_time(entry.unlockTime);
    }
    for (const ReplyGroup& group : decision.reply.groups) {
        auto* item = reply.add_groups();
        item->set_groupid(group.groupid);
        item->set_flags(1);
        item->set_unlocked(group.unlocked);
        item->set_complete(group.unlocked == group.total ? 1u : 0u);
        item->set_completed_at(group.completedAt);
    }
}

} // namespace ac::achieve
