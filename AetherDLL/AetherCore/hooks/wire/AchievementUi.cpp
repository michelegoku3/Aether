#include "pch.h"
#include "hooks/wire/AchievementUi.h"

#include <algorithm>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <system_error>
#include <utility>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "core/Workers.h"
#include "diagnostics/StatusWriter.h"
#include "hooks/wire/AchievementUserState.h"
#include "scripting/LuaData.h"
#include "steam_messages.pb.h"
#include "utils/WinFile.h"

namespace ac::achieve {
namespace {

constexpr const char* kModule = "AchieveUI";
constexpr std::int32_t kNoChange = -1;

// Bounds. A page open asks for one app, so these only have to cover the apps a
// session actually browses; the oldest entry leaves first (FIFO, not LRU: an
// entry is cheap to rebuild and the order of use is not worth tracking).
constexpr std::size_t kMaxSchemas = 64;
constexpr std::size_t kMaxPending = 64;
constexpr std::size_t kMaxStates = 64;

State& S() { return g_state.achievements.ui; }

std::uint32_t AccountIdFromSteamId(std::uint64_t steamId) {
    constexpr std::uint64_t kIndividualBase = 76561197960265728ull;
    if (steamId < kIndividualBase)
        return 0;
    const std::uint64_t id = steamId - kIndividualBase;
    return id > 0xFFFFFFFFull ? 0 : static_cast<std::uint32_t>(id);
}

std::string StatsDir() {
    return g_state.steamInstallPath.empty() ? std::string() : g_state.steamInstallPath + "\\appcache\\stats";
}

std::string UserBinPath(std::uint32_t accountId, std::uint32_t appId) {
    return StatsDir() + "\\UserGameStats_" + std::to_string(accountId) + "_" + std::to_string(appId) + ".bin";
}

std::uint64_t StateKey(std::uint32_t appId, std::uint32_t accountId) {
    return (static_cast<std::uint64_t>(appId) << 32) | accountId;
}

void SetReason(const char* reason) {
    {
        std::lock_guard<std::mutex> lock(S().mutex);
        S().lastReason = reason;
    }
    // status.json is a snapshot the writer only refreshes on request, and these
    // counters are the only way to see what the page decided without a log.
    ac::status::Write();
}

void RememberPending(std::uint64_t jobId, const Pending& pending) {
    State& s = S();
    std::lock_guard<std::mutex> lock(s.mutex);
    if (!s.pending.emplace(jobId, pending).second)
        return;
    s.pendingOrder.push_back(jobId);
    while (s.pendingOrder.size() > kMaxPending) {
        s.pending.erase(s.pendingOrder.front());
        s.pendingOrder.pop_front();
    }
}

bool TakePending(std::uint64_t jobId, Pending& out) {
    State& s = S();
    std::lock_guard<std::mutex> lock(s.mutex);
    const auto it = s.pending.find(jobId);
    if (it == s.pending.end())
        return false;
    out = it->second;
    s.pending.erase(it);
    const auto order = std::find(s.pendingOrder.begin(), s.pendingOrder.end(), jobId);
    if (order != s.pendingOrder.end())
        s.pendingOrder.erase(order);
    return true;
}

void StoreSchema(std::uint32_t appId, CapturedSchema schema) {
    State& s = S();
    std::lock_guard<std::mutex> lock(s.mutex);
    if (!s.schemas.contains(appId))
        s.schemaOrder.push_back(appId);
    s.schemas[appId] = std::move(schema);
    while (s.schemaOrder.size() > kMaxSchemas) {
        s.schemas.erase(s.schemaOrder.front());
        s.schemaOrder.pop_front();
    }
}

bool CopySchema(std::uint32_t appId, CapturedSchema& out) {
    State& s = S();
    std::lock_guard<std::mutex> lock(s.mutex);
    const auto it = s.schemas.find(appId);
    if (it == s.schemas.end())
        return false;
    out = it->second;
    return true;
}

bool CopyState(std::uint64_t key, CachedState& out) {
    State& s = S();
    std::lock_guard<std::mutex> lock(s.mutex);
    const auto it = s.states.find(key);
    if (it == s.states.end())
        return false;
    out = it->second;
    return true;
}

// Worker body: read the .bin, publish the parsed result. Disk and KV1 walking
// run here, never on a Steam thread.
void ReadStateNow(std::uint64_t key, std::uint32_t accountId, std::uint32_t appId) {
    CachedState fresh;
    const std::string path = UserBinPath(accountId, appId);
    std::error_code ec;
    const auto fsPath = winfile::ToPath(path);
    const std::uintmax_t size = std::filesystem::file_size(fsPath, ec);
    if (!ec && size > 0) {
        const auto stamp = std::filesystem::last_write_time(fsPath, ec);
        if (!ec)
            fresh.writeTime = static_cast<std::int64_t>(stamp.time_since_epoch().count());
        std::ifstream file(fsPath, std::ios::binary);
        if (file.is_open()) {
            std::vector<std::uint8_t> bytes((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
            UserState state;
            fresh.known = ParseUserState(bytes.data(), bytes.size(), state);
            fresh.size = bytes.size();
            fresh.unlocked = std::move(state.unlocked);
        }
    }

    State& s = S();
    {
        std::lock_guard<std::mutex> lock(s.mutex);
        s.states[key] = std::move(fresh);
        if (s.states.size() > kMaxStates) {
            // Drop the entry that is not this one and was inserted earliest.
            // The map has no order, so a state that is not the current key may
            // leave: the next page open re-reads it.
            for (auto it = s.states.begin(); it != s.states.end(); ++it) {
                if (it->first != key) {
                    s.states.erase(it);
                    break;
                }
            }
        }
        s.refreshing.erase(key);
    }
    // The cached-state count is part of status.json; publish the first read.
    ac::status::Write();
}

// Schedules a read for (app, account). At most one is in flight per pair, so a
// page refresh loop cannot queue unbounded work.
void RequestStateRead(std::uint32_t appId, std::uint32_t accountId) {
    if (accountId == 0 || g_state.steamInstallPath.empty())
        return;
    const std::uint64_t key = StateKey(appId, accountId);
    {
        State& s = S();
        std::lock_guard<std::mutex> lock(s.mutex);
        if (!s.refreshing.insert(key).second)
            return;
    }
    const bool queued = workers::Submit([key, accountId, appId] { ReadStateNow(key, accountId, appId); });
    if (!queued) {
        // Shutdown: leave the cache cold, the next page open is pass-through.
        std::lock_guard<std::mutex> lock(S().mutex);
        S().refreshing.erase(key);
    }
}

} // namespace

bool Enabled() {
    const auto settings = Settings::Snapshot();
    return settings && settings->achievementsUiService;
}

bool OnSendSchema(std::uint64_t jobId, const std::uint8_t* body, std::uint32_t len) {
    if (!Enabled())
        return false;
    CPlayer_GetGameAchievements_Request request;
    if (!body || !request.ParseFromArray(body, static_cast<int>(len)))
        return false;
    S().calls.fetch_add(1, std::memory_order_relaxed);
    ac::status::Write();
    Pending pending;
    pending.userCall = false;
    pending.appId = request.appid();
    RememberPending(jobId, pending);
    return true;
}

bool OnSendUser(std::uint64_t jobId, const std::uint8_t* body, std::uint32_t len) {
    if (!Enabled())
        return false;
    CPlayer_GetUserAchievements_Request request;
    if (!body || !request.ParseFromArray(body, static_cast<int>(len)))
        return false;
    S().calls.fetch_add(1, std::memory_order_relaxed);
    ac::status::Write();

    Pending pending;
    pending.userCall = true;
    pending.appId = request.appid();
    pending.steamId = request.steamid();
    RememberPending(jobId, pending);

    // The read starts now so it is usually done by the time the reply lands.
    // The account must come from the request: resolving it from the registry or
    // loginusers.vdf would be disk I/O on a Steam network thread. A request
    // that names no account is left entirely to Steam.
    if (ac::luadata::HasDepot(pending.appId))
        RequestStateRead(pending.appId, AccountIdFromSteamId(pending.steamId));
    return true;
}

std::int32_t OnRecvSchema(std::uint64_t jobId, const std::uint8_t* body, std::uint32_t len) {
    Pending pending;
    if (!TakePending(jobId, pending) || pending.userCall)
        return kNoChange;
    CPlayer_GetGameAchievements_Response reply;
    if (!body || !reply.ParseFromArray(body, static_cast<int>(len))) {
        SetReason("schema-unparsed");
        return kNoChange;
    }

    // A hash-only or empty reply must never wipe a good capture: the client
    // asks with hash_only=1 when it only wants to invalidate, and the next real
    // reply carries the entries again.
    if (reply.achievements_size() == 0) {
        SetReason("schema-empty");
        return kNoChange;
    }

    CapturedSchema schema;
    schema.version = reply.schema_version();
    schema.hash = reply.schema_hash();
    schema.entries.reserve(static_cast<std::size_t>(reply.achievements_size()));
    for (const auto& achievement : reply.achievements()) {
        SchemaEntry entry;
        entry.hasKey = achievement.has_internal_key();
        entry.key = achievement.internal_key();
        entry.groupid = achievement.groupid();
        entry.name = achievement.internal_name();
        schema.entries.push_back(std::move(entry));
    }
    const std::size_t count = schema.entries.size();
    const std::uint32_t hash = schema.hash;
    const std::int32_t version = schema.version;
    StoreSchema(pending.appId, std::move(schema));
    S().schemasCaptured.fetch_add(1, std::memory_order_relaxed);
    ac::status::Write();
    AC_LOG_DEBUG(kModule, "App %u: captured %zu achievement(s), schema v%d hash %08X.", pending.appId, count, version,
                 hash);
    // The schema reply is never rewritten: the client renders names, icons and
    // descriptions from it and Aether has nothing better to offer.
    return kNoChange;
}

std::int32_t OnRecvUser(std::uint64_t jobId, std::int32_t eresult, const std::uint8_t* body, std::uint32_t len,
                        std::uint8_t* out, std::uint32_t cap) {
    Pending pending;
    if (!TakePending(jobId, pending) || !pending.userCall)
        return kNoChange;
    if (!Enabled()) {
        SetReason("off");
        return kNoChange;
    }
    // Only a successful reply is a candidate: an error body carries no state to
    // complete, and rewriting it would replace the reason the client shows.
    if (eresult != 1) { // 1 = k_EResultOK
        S().passthrough.fetch_add(1, std::memory_order_relaxed);
        SetReason("eresult");
        return kNoChange;
    }

    CPlayer_GetUserAchievements_Response reply;
    if (!body || !reply.ParseFromArray(body, static_cast<int>(len))) {
        S().passthrough.fetch_add(1, std::memory_order_relaxed);
        SetReason("reply-unparsed");
        return kNoChange;
    }

    Input in;
    if (!ac::luadata::HasDepot(pending.appId)) {
        S().passthrough.fetch_add(1, std::memory_order_relaxed);
        // Name the reason the page is Valve's: a genuine licence, a family
        // licence, or an app Aether does not manage at all.
        SetReason(ac::luadata::IsFamilyShared(pending.appId) ? "family-shared"
                  : ac::luadata::IsOwned(pending.appId)      ? "owned"
                                                             : "not-managed");
        return kNoChange;
    }
    in.managed = true;
    in.familyShared = ac::luadata::IsFamilyShared(pending.appId);

    // The rewrite exists for the empty body the CM sends for an unowned app. A
    // reply that already carries state is Valve's own answer, progress fields
    // included, and is worth more than anything rebuilt from the .bin.
    if (reply.achievements_size() != 0 || reply.groups_size() != 0) {
        S().passthrough.fetch_add(1, std::memory_order_relaxed);
        SetReason("steam-state");
        return kNoChange;
    }

    const std::uint32_t accountId = AccountIdFromSteamId(pending.steamId);
    if (accountId == 0) {
        S().passthrough.fetch_add(1, std::memory_order_relaxed);
        SetReason("no-account");
        return kNoChange;
    }

    CapturedSchema schema;
    const bool haveSchema = CopySchema(pending.appId, schema);
    in.schemaKnown = haveSchema && !schema.entries.empty();
    in.entries = haveSchema ? schema.entries : std::vector<SchemaEntry>{};
    // Echo the pair the client just sent; fall back to the captured schema only
    // when the reply omitted them. The hash is not derivable, so it is never
    // invented.
    in.version = reply.schema_version() != 0 ? reply.schema_version() : schema.version;
    in.hash = reply.schema_hash() != 0 ? reply.schema_hash() : schema.hash;

    CachedState state;
    in.stateKnown = CopyState(StateKey(pending.appId, accountId), state) && state.known;
    if (in.stateKnown)
        in.unlocked = std::move(state.unlocked);
    // Keep the cache warm for the next open, and pick up an unlock earned since
    // the last read; the rewrite below answers from what is cached right now.
    RequestStateRead(pending.appId, accountId);

    const Decision decision = Decide(in);
    if (decision.verdict != Verdict::Rebuild) {
        S().passthrough.fetch_add(1, std::memory_order_relaxed);
        SetReason(decision.reason);
        AC_LOG_DEBUG(kModule, "App %u: page left to Steam (%s).", pending.appId, decision.reason);
        return kNoChange;
    }

    // Invariant: only the two user-owned blocks change. Everything else in
    // Valve's body (including fields a future client adds) is re-emitted.
    FillReply(reply, decision);

    const std::size_t size = reply.ByteSizeLong();
    if (size > cap) {
        S().passthrough.fetch_add(1, std::memory_order_relaxed);
        SetReason("too-large");
        AC_LOG_WARN_ONCE(kModule, "Rebuilt reply for app %u needs %zu byte (cap %u); passing Steam's through.",
                         pending.appId, size, cap);
        return kNoChange;
    }
    if (!reply.SerializeToArray(out, static_cast<int>(cap)))
        return kNoChange;

    S().rebuilt.fetch_add(1, std::memory_order_relaxed);
    SetReason("rebuilt");
    AC_LOG_DEBUG(kModule, "App %u: %zu unlock(s) in %zu group(s) rebuilt from local state (%zu byte).", pending.appId,
                 decision.reply.entries.size(), decision.reply.groups.size(), size);
    return static_cast<std::int32_t>(size);
}

} // namespace ac::achieve
