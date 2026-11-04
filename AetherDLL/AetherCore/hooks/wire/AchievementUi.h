#pragma once

#include <atomic>
#include <chrono>
#include <cstdint>
#include <deque>
#include <mutex>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include "core/SteamTypes.h"
#include "hooks/wire/AchievementStatsSchema.h"
#include "hooks/wire/AchievementUiPolicy.h"

// ---------------------------------------------------------------------------
// Client achievement page - service path glue.
//
// The Steam client asks the CM for a game's achievement schema
// (Player.GetGameAchievements#1) and for the account's achievement state
// (Player.GetUserAchievements#1), then renders the page from both. For an app
// the account does not own (every app Aether advertises) the CM still answers
// the schema but sends an empty state body, so the page shows no achievements
// even though the local .bin holds the real unlocks.
//
// This module remembers the schema from the reply the client already receives,
// reads the local state on a worker, and rebuilds the state reply. The schema
// reply always passes through untouched.
//
// Two things make the page appear on the first open instead of the second:
//   * a reply whose schema or local state is not ready yet is held (the client
//     gets a copy aimed at a job nobody waits on, so its real job stays
//     pending) and delivered once the data lands, with an 8 s safety net;
//   * the rebuilt body lists every schema achievement, not only unlocked ones;
//   * each entry carries the counter from the local stats file, so a page row
//     shows the same progress the overlay does.
//
// All entry points take the job id and raw bodies: the wire layer owns frame
// decoding, this module owns the protobuf payloads. Nothing here touches disk
// on the caller's thread; per-call and cache state live in
// AetherCoreState::AchievementStore::ui (core rule, docs/ARCHITECTURE.md).
// ---------------------------------------------------------------------------
namespace ac::achieve {

// Schema captured from a GetGameAchievements reply the client received.
struct CapturedSchema {
    std::int32_t version = 0;
    std::uint32_t hash = 0;
    std::vector<SchemaEntry> entries;
    std::vector<std::uint32_t> groups;
};

// Local state for one (app, account), read on a worker from
// <steam>\appcache\stats\UserGameStats_<account>_<appid>.bin plus the game
// schema that names each progress counter (AchievementStatsSchema.h).
// `loaded` says a read finished; `known` says the stats file existed and parsed,
// so an absent file answers "nothing unlocked, no progress" instead of waiting
// forever.
struct CachedState {
    bool loaded = false;
    bool known = false;
    std::uintmax_t size = 0; // bytes read
    std::int64_t writeTime = 0;
    std::unordered_map<std::uint32_t, std::uint32_t> unlocked;
    std::unordered_map<std::uint32_t, float> progress; // key -> counter value
};

// The game schema's progress map, kept per app. Steam rewrites
// UserGameStatsSchema_<appid>.bin when the game's schema changes, so the cached
// copy is re-read when its size or write time moves; a missing file is not
// cached, and the next page open tries again.
struct CachedStatsSchema {
    std::uintmax_t size = 0;
    std::int64_t writeTime = 0;
    StatsSchema schema;
};

// One outstanding service call, remembered from its send so the reply can be
// attributed (responses carry only jobid_target).
struct Pending {
    bool userCall = false; // false: GetGameAchievements
    std::uint32_t appId = 0;
    std::uint64_t steamId = 0; // as sent; 0 when the request named no account
};

// A user reply that could not be answered yet: the header already carries
// eresult OK and the real jobid_target, the body is Valve's original.
struct HeldReply {
    std::uint32_t appId = 0;
    std::uint32_t accountId = 0;
    std::string header;
    std::string body;
    std::chrono::steady_clock::time_point at;
};

// Shared state: wire threads read and write, workers fill the state cache, the
// status writer reads the counters. One mutex, held only for map operations
// and short copies - never across I/O, protobuf parsing or logging.
struct State {
    mutable std::mutex mutex;
    std::unordered_map<std::uint32_t, CapturedSchema> schemas; // bounded by schemaOrder
    std::deque<std::uint32_t> schemaOrder;
    std::unordered_map<std::uint64_t, Pending> pending;
    std::deque<std::uint64_t> pendingOrder;
    std::unordered_map<std::uint64_t, CachedState> states;             // (appId << 32) | accountId
    std::unordered_map<std::uint32_t, CachedStatsSchema> statsSchemas; // bounded by statsSchemaOrder
    std::deque<std::uint32_t> statsSchemaOrder;
    std::unordered_set<std::uint64_t> refreshing;
    std::vector<HeldReply> held;
    std::deque<std::vector<std::uint8_t>> outbox;  // complete 147 frames ready to deliver
    std::atomic<std::uint64_t> calls{0};           // send seen (both methods)
    std::atomic<std::uint64_t> schemasCaptured{0}; // schemas stored
    std::atomic<std::uint64_t> rebuilt{0};         // user replies answered from local state
    std::atomic<std::uint64_t> passthrough{0};     // user replies left alone
    std::atomic<std::uint64_t> heldCount{0};       // replies parked waiting for data
    std::atomic<std::uint64_t> delivered{0};       // held replies delivered later
    std::atomic<std::uint64_t> progressValues{0};  // entries with a counter in the last rebuild
    std::string lastReason = "idle";               // guarded by mutex
};

// Wire entry points. Send handlers only remember the call and warm the state
// cache; the reply handlers return a new body length or -1 for pass-through.
// OnRecvSchema never rewrites the body, it only learns the schema. OnRecvUser
// always forces eresult OK (the app is not denied, it just has no data) and,
// when it has to hold the reply, aims the copy at a job nobody waits on.
bool OnSendSchema(std::uint64_t jobId, const std::uint8_t* body, std::uint32_t len);
bool OnSendUser(std::uint64_t jobId, const std::uint8_t* body, std::uint32_t len);
std::int32_t OnRecvSchema(std::uint64_t jobId, const std::uint8_t* body, std::uint32_t len);
std::int32_t OnRecvUser(std::uint64_t jobId, const std::uint8_t* header, std::uint32_t headerLen,
                        const std::uint8_t* body, std::uint32_t len, std::uint8_t* outHeader,
                        std::uint32_t outHeaderCap, std::int32_t* outHeaderLen, std::uint8_t* out, std::uint32_t cap);

// Delivers one queued reply and releases held replies whose data never arrived.
// Called by the RecvPkt hook before it dispatches the current packet, so a frame
// queued during a dispatch goes out on the next one.
void TryDeliver(void* recvThis, steam::CNetPacket* carrier, void* (*oRecvPkt)(void*, steam::CNetPacket*));

// True when the settings snapshot enables the page rewrite.
bool Enabled();

} // namespace ac::achieve
