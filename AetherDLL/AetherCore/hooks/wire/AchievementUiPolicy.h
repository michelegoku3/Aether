#pragma once

#include <cstdint>
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
// model. It is free of logs, g_state and Windows so the decision table can be
// tested directly. `FillReply` writes the model into a response message; the
// message type is a template parameter so no protobuf type is declared here.
//
// The shape was verified against the client: one
// entry per schema achievement (locked ones included), `progress_float` always
// present, and one `is_achievable` group per group the schema lists. The page
// draws its rows from this reply, so a locked achievement with no entry, or a
// reply with no entries at all, leaves the section blank. `progress_float` is
// the row's counter: an entry without it, or with a stale 0, draws an empty bar
// even when the game recorded progress.
// ---------------------------------------------------------------------------
namespace ac::achieve {

// One achievement of the captured schema (GetGameAchievements reply).
struct SchemaEntry {
    std::uint32_t key = 0; // internal_key = bucket * 256 + bit
    std::uint32_t groupid = 0;
};

struct ReplyEntry {
    std::uint32_t key = 0;
    bool unlocked = false;
    std::uint32_t unlockTime = 0; // 0 when the bit is set without a recorded date
    float progress = 0.0f;        // raw stat value; 0 when the game keeps none
};

struct ReplyGroup {
    std::uint32_t groupid = 0;
};

struct Reply {
    std::vector<ReplyEntry> entries; // schema order
    std::vector<ReplyGroup> groups;  // schema order
};

enum class Verdict {
    Passthrough, // leave Valve's frame untouched
    Hold,        // schema or local state not ready: answer this reply later
    Rebuild      // replace the two blocks this module owns
};

struct Input {
    bool managed = false;     // configured .lua app, not genuinely owned
    bool schemaKnown = false; // a GetGameAchievements reply was captured
    bool stateLoaded = false; // the local .bin read finished (the file may be absent)
    std::vector<SchemaEntry> entries;
    std::vector<std::uint32_t> groups;
    std::unordered_map<std::uint32_t, std::uint32_t> unlocked; // key -> time
    std::unordered_map<std::uint32_t, float> progress;         // key -> counter value
};

struct Decision {
    Verdict verdict = Verdict::Passthrough;
    const char* reason = "";
    Reply reply;
};

// Decision table, in the order the guards apply. Every "pass" leaves Valve's
// frame untouched; "hold" keeps the client's job pending until the data it
// needs exists (the caller delivers the rebuilt reply through its own outbox).
inline Decision Decide(const Input& in) {
    Decision decision;
    if (!in.managed) {
        decision.reason = "not-managed";
        return decision;
    }
    if (!in.schemaKnown || in.entries.empty()) {
        decision.verdict = Verdict::Hold;
        decision.reason = "no-schema";
        return decision;
    }
    if (!in.stateLoaded) {
        decision.verdict = Verdict::Hold;
        decision.reason = "no-state";
        return decision;
    }

    decision.reply.entries.reserve(in.entries.size());
    for (const SchemaEntry& entry : in.entries) {
        ReplyEntry out;
        out.key = entry.key;
        const auto it = in.unlocked.find(entry.key);
        if (it != in.unlocked.end()) {
            out.unlocked = true;
            out.unlockTime = it->second;
        }
        const auto progress = in.progress.find(entry.key);
        if (progress != in.progress.end())
            out.progress = progress->second;
        decision.reply.entries.push_back(out);
    }
    decision.reply.groups.reserve(in.groups.size());
    for (const std::uint32_t groupid : in.groups)
        decision.reply.groups.push_back(ReplyGroup{groupid});

    decision.verdict = Verdict::Rebuild;
    decision.reason = "rebuilt";
    return decision;
}

// Writes the decision's model into a response message: the two blocks Aether
// owns, and nothing else, so `schema_version`/`schema_hash` and every unknown
// field Valve sent are re-emitted. An entry is emitted for every achievement,
// locked or not, because the page reads its rows from this reply.
template <typename Response> void FillReply(Response& reply, const Decision& decision) {
    reply.clear_achievements();
    reply.clear_groups();
    for (const ReplyEntry& entry : decision.reply.entries) {
        auto* item = reply.add_achievements();
        item->set_internal_key(entry.key);
        if (entry.unlocked) {
            item->set_unlocked(true);
            item->set_unlock_time(entry.unlockTime);
        }
        // Presence matters more than the value: an entry without it leaves the
        // row blank on the page.
        item->set_progress_float(entry.progress);
    }
    for (const ReplyGroup& group : decision.reply.groups) {
        auto* item = reply.add_groups();
        item->set_groupid(group.groupid);
        item->set_is_achievable(true);
    }
}

} // namespace ac::achieve
