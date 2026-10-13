#include "pch.h"
#include "hooks/wire/AchievementModule.h"

#include <atomic>
#include <bit>
#include <chrono>
#include <ctime>
#include <mutex>
#include <string>
#include <unordered_map>

#include "core/AetherCoreState.h"
#include "hooks/aetheronline/PresenceSession.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "credentials/SteamId.h"
#include "hooks/wire/AchievementBackup.h"
#include "hooks/wire/DonorPool.h"
#include "scripting/LuaData.h"
#include "steam_messages.pb.h"
#include "utils/Hasher.h"

// ============================================================================
// Achievement / UserStats spoofing — ported to behave like LumaCore.
//
// Differences vs the previous Aether implementation:
//   1. The OpenSteamTool donor API (stats.opensteamtool.com) has been REMOVED.
//      It never returned usable data, so we no longer block/time out on it.
//   2. The `sha_schema` short-circuit gate has been removed: when the game
//      sends a GetUserStats request that already carries a local schema, we
//      now CLEAR it and spoof anyway (exactly what LumaCore does). This was
//      the bug that broke achievements for games that send a cached schema.
//   3. Donor selection uses LumaCore's self-learning per-app pool: we try the
//      fixed donor pool and REMEMBER which donor responds with real schema /
//      stat / achievement data, so subsequent requests reuse it. If a donor
//      stops returning data we advance to the next pool entry.
//
// ----------------------------------------------------------------------------
// LOGGING, MIRROR AND BACKUP (August 2026 debug session — achievement loss):
//
// The Steam client keeps achievements ONLY in the local cache
// (<steam>\appcache\stats\UserGameStats_<account>_<appid>.bin): the server
// rejects StoreUserStats2 from accounts that do not own the game, so that
// cache is the ONLY copy of progress and any client-side invalidation is a
// permanent loss (see also the "PendingChanges" field in the binary file:
// changes never confirmed by the server).
//
// That is why every stage of the flow leaves a detailed trace in the log and,
// above all, every unlock is replicated into AetherDesk's Backup section
// (when installed, via desk_path.cfg) with the same Steam naming convention —
// one file per (account, appid). Persistence lives in the dedicated module
// hooks/wire/AchievementBackup (separation of concerns): RecordUnlock() is
// asynchronous, no disk I/O on Steam's network thread.
//
//   <AetherData>\backup\<appid>\achievements\            [via desk_path.cfg]
//   <steam>\aethercore\backup\<appid>\achievements\      [fallback]
//       UserGameStats_<account>_<appid>.json   unlock snapshot (readable,
//                                               regenerates the .bin with the tool)
//       UserGameStats_<account>_<appid>.bin    copy of the Steam cache
//       UserGameStatsSchema_<appid>.bin        copy of the game schema
//
// If Steam's cache were wiped again, everything is recoverable from the backup:
// either by putting the .bins back in place (Steam closed), or by rebuilding the
// .bin from JSON with Tools/achievement_decoder.py (the `rebuild` command).
//
// Flusso tracciato (con eMsg):
//   151  Send  Player.GetUserStats (service)          -> donor spoofing
//   152  Recv  Player.GetUserStats response (147)      -> donor data cleanup
//   818  Send  ClientGetUserStats                      -> donor spoofing
//   819  Recv  ClientGetUserStatsResponse              -> donor data cleanup
//   5466 Send  ClientStoreUserStats2                   -> achievement UNLOCK
// ============================================================================

namespace ac::hooks::AchievementModule {
namespace {

constexpr const char* kModule = "Wire.Achievement";
constexpr std::int32_t kNoChange = -1;
// Donor pool, attempt correlation and pending 818s live in hooks/wire/DonorPool
// (P9 extraction): only pure decoders and wire handlers remain here.
using DonorPool::Correl;
using DonorPool::StatAttempt;
using Clock = DonorPool::Clock;

// Last known value of the bitfield stats per app (stat_id -> value), to
// detect NEW bits on every commit (achievements via stats).
std::mutex g_statBitsMutex;
std::unordered_map<steam::AppId, std::unordered_map<std::uint32_t, std::uint32_t>> g_statBits;

// Active SteamID cache: GetActiveSteamId64 reads registry/filesystem on
// every call, but 5466 can arrive many times per minute. The account never
// changes without a Steam restart: 30s TTL (freshness-vs-cost tradeoff),
// lock-free atomics (a 64-bit value is atomic on x64).
std::atomic<std::uint64_t> g_cachedActiveSteamId{0};
std::atomic<Clock::rep> g_cachedActiveSteamIdAt{0};
constexpr auto kSteamIdCacheTtl = std::chrono::seconds(30);

std::uint64_t CachedActiveSteamId64() {
    const auto now = Clock::now().time_since_epoch().count();
    const std::uint64_t cached = g_cachedActiveSteamId.load(std::memory_order_relaxed);
    if (cached != 0 && now - g_cachedActiveSteamIdAt.load(std::memory_order_relaxed) < kSteamIdCacheTtl.count()) {
        return cached;
    }
    const std::uint64_t resolved = ac::steamid::GetActiveSteamId64();
    if (resolved != 0) {
        g_cachedActiveSteamId.store(resolved, std::memory_order_relaxed);
        g_cachedActiveSteamIdAt.store(now, std::memory_order_relaxed);
    }
    return resolved;
}

// Single serialization queue for all handlers (P9): previously 8 nearly
// identical "ByteSizeLong + cap + SerializeToArray" blocks. Non-null warnTag
// = log the failure (send paths); null = silence (recv).
template <class TMsg>
std::int32_t SerializeTo(const TMsg& msg, std::uint8_t* out, std::uint32_t outCap,
                         const char* warnTag) {
    const std::uint32_t size = static_cast<std::uint32_t>(msg.ByteSizeLong());
    if (size > outCap || !msg.SerializeToArray(out, static_cast<int>(outCap))) {
        if (warnTag) {
            AC_LOG_WARN(kModule, "%s serialization failed (size=%u, cap=%u): frame unchanged.",
                        warnTag, size, outCap);
        }
        return kNoChange;
    }
    return static_cast<std::int32_t>(size);
}

bool IsOK(std::int32_t eresult) { return eresult == 1; } // k_EResultOK

bool HasStatsPayload(const CPlayer_GetUserStats_Response& resp) {
    return (resp.has_schema() && !resp.schema().empty()) || resp.stats_size() > 0;
}

bool HasStatsPayload(const CMsgClientGetUserStatsResponse& resp) {
    return (resp.has_schema() && !resp.schema().empty())
        || resp.stats_size() > 0
        || resp.achievement_blocks_size() > 0;
}

// ---------------------------------------------------------------------------
// AetherOnline session resolution (single source of truth for this module).
//
// With the "-aetheronline" launch flag the game process is masked as Spacewar
// (AppID 480): every UserStats frame it sends carries appid/game_id == 480,
// and the Lua-managed gate below (HasDepot) would reject it because depot
// keys are registered under the REAL app id. Rewriting the frame to the real
// app id restores the donor-spoofing pipeline for AetherOnline sessions.
//
// The real app id is captured at spawn time by AetherOnlineHooks::h_SpawnProcess
// into the PresenceSession snapshot. When no session is active (or the id is
// already the real one) the frame is left untouched.
//
// Returns true when 'appId' was rewritten, so callers can mirror the new id
// back into their protobuf request (the 819 response path has no field to
// mirror — it only uses the resolved id for correlation/gating).
bool ResolveAetherOnlineAppId(steam::AppId& appId, const char* flow) {
    const steam::AppId real = presence::RealAppId();
    if (appId == constants::kSpacewarAppId && real != 0 && real != constants::kSpacewarAppId) {
        AC_LOG_INFO_ONCE(kModule, "AetherOnline: %s AppID %u -> %u.", flow, appId, real);
        appId = real;
        return true;
    }
    return false;
}

} // namespace

// ---------------------------------------------------------------------------
// Outgoing Request Interception and Spoofing (HandleSend)
// ---------------------------------------------------------------------------

std::int32_t HandleSendGetUserStats(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CPlayer_GetUserStats_Request req;
    if (!req.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        AC_LOG_WARN(kModule, "[151 GetUserStats] Parse failed (bodyLen=%u): frame unchanged.", frame.bodyLen);
        return kNoChange;
    }

    steam::AppId appId = req.appid();

    // AetherOnline: the request may arrive masked as 480 (see
    // ResolveAetherOnlineAppId) — rewrite the frame before the Lua-managed gate.
    if (ResolveAetherOnlineAppId(appId, "GetUserStats")) {
        req.set_appid(appId);
    }

    if (!ac::luadata::HasDepot(appId)) {
        AC_LOG_DEBUG(kModule, "[151 GetUserStats] AppID %u not configured (HasDepot=false): frame unchanged.", appId);
        return kNoChange;
    }

    const std::uint64_t originalSteamId = req.steamid();

    AC_LOG_DEBUG(kModule,
                "[151 GetUserStats] Outgoing request for AppID %u: requested steamid=%llu, crc_stats=%u, "
                "sha_schema present=%s (%u byte, fnv1a %016llx).",
                appId,
                static_cast<unsigned long long>(originalSteamId),
                req.crc_stats(),
                req.has_sha_schema() && !req.sha_schema().empty() ? "yes" : "no",
                req.has_sha_schema() ? static_cast<std::uint32_t>(req.sha_schema().size()) : 0u,
                (ac::log::Enabled(LogLevel::Debug) && req.has_sha_schema())
                    ? static_cast<unsigned long long>(
                          ac::hasher::Fnv1a64(req.sha_schema().data(), req.sha_schema().size()))
                    : 0ull);

    // LumaCore behaviour: clear any local schema and spoof anyway. The old
    // code bailed out here when the request already carried a sha_schema,
    // which is exactly why achievements failed for games that send a cached
    // schema (e.g. Endacopia / AppID 2684630).
    req.clear_sha_schema();

    const std::size_t poolIndex = DonorPool::PickIndex(appId);
    const std::uint64_t donorId = DonorPool::DonorSteamId(poolIndex);

    // Correlate with the response so we can learn from the donor result.
    StatAttempt attempt;
    attempt.appId = appId;
    attempt.poolIndex = poolIndex;
    CMsgProtoBufHeader hdr;
    bool hasJobId = false;
    std::uint64_t jobId = 0;
    if (hdr.ParseFromArray(frame.header, static_cast<int>(frame.headerLen)) && hdr.has_jobid_source()) {
        hasJobId = true;
        jobId = hdr.jobid_source();
    }
    DonorPool::RecordAttempt(attempt, hasJobId, jobId);

    req.set_steamid(donorId);

    const std::int32_t serialized = SerializeTo(req, out, outCap, "[151 GetUserStats]");
    if (serialized == kNoChange) return kNoChange;

    AC_LOG_INFO_ONCE(kModule, "[151 GetUserStats] Spoofing AppID %u: steamid %llu -> donor %llu (pool %zu, jobid %s).",
                appId, static_cast<unsigned long long>(originalSteamId), donorId, poolIndex, hasJobId ? "yes" : "no");
    return serialized;
}

std::int32_t HandleSendClientGetUserStats(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CMsgClientGetUserStats req;
    if (!req.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        AC_LOG_WARN(kModule, "[818 ClientGetUserStats] Parse failed (bodyLen=%u): frame unchanged.", frame.bodyLen);
        return kNoChange;
    }

    steam::AppId appId = static_cast<steam::AppId>(req.game_id());

    // AetherOnline: the request may arrive masked as 480 (see
    // ResolveAetherOnlineAppId) — rewrite the frame before the Lua-managed gate.
    if (ResolveAetherOnlineAppId(appId, "ClientGetUserStats")) {
        req.set_game_id(appId);
    }

    if (!ac::luadata::HasDepot(appId)) {
        AC_LOG_DEBUG(kModule, "[818 ClientGetUserStats] AppID %u not configured (HasDepot=false): frame unchanged.", appId);
        return kNoChange;
    }

    const std::size_t poolIndex = DonorPool::PickIndex(appId);
    const std::uint64_t donorId = DonorPool::DonorSteamId(poolIndex);
    const std::uint64_t originalUserId = req.steam_id_for_user();

    // Session backup: on the app's first 818, copy the existing .bin cache
    // (protects already-accumulated progress even with no new unlocks).
    AchievementBackup::TouchSession(appId, originalUserId);

    AC_LOG_DEBUG(kModule,
                "[818 ClientGetUserStats] Outgoing request for AppID %u: steam_id_for_user=%llu, "
                "crc_stats=%u, schema_local_version=%d -> spoofing with donor %llu (pool %zu), "
                "crc_stats cleared and schema_local_version=-1.",
                appId, static_cast<unsigned long long>(originalUserId), req.crc_stats(),
                req.schema_local_version(), donorId, poolIndex);

    // Force the schema request by zeroing the local version (fetches the
    // updated schema) and the local crc (like LumaCore).
    req.clear_crc_stats();
    req.set_schema_local_version(-1);

    req.set_steam_id_for_user(donorId);

    StatAttempt attempt;
    attempt.appId = appId;
    attempt.poolIndex = poolIndex;
    attempt.seen = Clock::now();   // FIX (21/08 log): without this the default
                                   // is epoch → TakePendingClientStats' prune
                                   // treated the entry as already expired and
                                   // deleted it: EVERY 819 came out
                                   // "uncorrelated" (and with a data-carrying
                                   // donor, a leak).
    DonorPool::RecordPendingClientStats(appId, attempt);

    const std::int32_t serialized = SerializeTo(req, out, outCap, "[818 ClientGetUserStats]");
    if (serialized == kNoChange) return kNoChange;

    AC_LOG_DEBUG(kModule, "[818 ClientGetUserStats] Spoofing AppID %u with DonorID %llu (pool %zu): rewritten and forwarded.",
                 appId, donorId, poolIndex);
    return serialized;
}

// ---------------------------------------------------------------------------
// Incoming Response Normalization (HandleRecv)
// ---------------------------------------------------------------------------

std::int32_t HandleRecvGetUserStatsResponse(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap,
                                            std::uint8_t* outHdr, std::uint32_t outHdrCap, std::int32_t* outNewHdrLen) {
    // Resolve which (spoofed) request this response belongs to.
    CMsgProtoBufHeader hdrMsg;
    if (!hdrMsg.ParseFromArray(frame.header, static_cast<int>(frame.headerLen))) {
        return kNoChange;
    }
    StatAttempt attempt;
    const Correl correl = DonorPool::ResolveAttempt(hdrMsg.has_jobid_target(), hdrMsg.jobid_target(), attempt);
    if (correl == Correl::NoMatch) {
        AC_LOG_DEBUG(kModule,
                    "[152 GetUserStats Response] No correlated attempt: response passed through unchanged "
                    "(eresult original=%d).",
                    hdrMsg.has_eresult() ? hdrMsg.eresult() : -1);
        return kNoChange;
    }

    // Learn which donor actually returned useful data.
    CPlayer_GetUserStats_Response resp;
    if (!resp.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        AC_LOG_WARN(kModule, "[152 GetUserStats Response] Body parse failed (bodyLen=%u): response unchanged.",
                    frame.bodyLen);
        return kNoChange;
    }

    // DONOR response not precisely correlatable: Ambiguous (multiple spoofed
    // requests in flight) or Resolved onto an app that became unmanaged after
    // the send (e.g. MarkOwned fired in the meantime). In both cases the
    // payload belongs to the donor and must NOT reach the game (the "The Fool"
    // leak: donor achievements delivered to the user). We strip stats and crc,
    // keep the schema, header unchanged.
    if (correl == Correl::Ambiguous || !ac::luadata::HasDepot(attempt.appId)) {
        const int removedStats = resp.stats_size();
        const std::uint32_t schemaBytes = resp.has_schema() ? static_cast<std::uint32_t>(resp.schema().size()) : 0u;
        resp.clear_stats();
        resp.clear_crc_stats();
        const std::int32_t serialized = SerializeTo(resp, out, outCap, nullptr);
        if (serialized == kNoChange) return kNoChange;
        AC_LOG_WARN(kModule,
                    "[152 GetUserStats Response] Donor response %s: %d stats removed for safety, "
                    "schema preserved (%u byte). No leak to the game.",
                    correl == Correl::Ambiguous ? "not correlatable (overlapping requests)"
                                                : "for an app no longer managed",
                    removedStats, schemaBytes);
        return serialized;
    }
    const std::int32_t originalResult = hdrMsg.has_eresult() ? hdrMsg.eresult() : -1;
    const bool okWithData = IsOK(originalResult) && HasStatsPayload(resp);
    const int donorStatsCount = resp.stats_size();
    const std::uint32_t schemaSize = resp.has_schema() ? static_cast<std::uint32_t>(resp.schema().size()) : 0u;

    AC_LOG_DEBUG(kModule,
                "[152 GetUserStats Response] AppID %u donor %llu (pool %zu): eresult=%d, schema=%u byte "
                "(fnv1a %016llx, sha_schema %s), stats=%d, crc_stats=%u -> donor %s.",
                attempt.appId, DonorPool::DonorSteamId(attempt.poolIndex), attempt.poolIndex, originalResult,
                resp.has_schema() ? static_cast<std::uint32_t>(resp.schema().size()) : 0u,
                (ac::log::Enabled(LogLevel::Debug) && resp.has_schema())
                    ? static_cast<unsigned long long>(ac::hasher::Fnv1a64(resp.schema().data(), resp.schema().size()))
                    : 0ull,
                (resp.has_sha_schema() && !resp.sha_schema().empty()) ? "present" : "absent",
                resp.stats_size(), resp.crc_stats(), okWithData ? "WITH data" : "WITHOUT data");

    // Detail of the donor data about to be dropped (debug).
    // The loop is skipped entirely when Debug is off: the donor can carry
    // dozens of stats and log-time parsing is not free.
    if (ac::log::Enabled(LogLevel::Debug)) for (const auto& st : resp.stats()) {
        if (st.unlock_times_size() > 0) {
            for (const auto& ut : st.unlock_times()) {
                AC_LOG_DEBUG(kModule,
                            "[152 GetUserStats Response] Discarded donor data: stat_id=%u achievement_bit=%u "
                            "unlock_time=%u (%s local).",
                            st.stat_id(), ut.achievement_bit(), ut.unlock_time(),
                            AchievementBackup::FormatUnixTime(ut.unlock_time()).c_str());
            }
        } else {
            AC_LOG_DEBUG(kModule, "[152 GetUserStats Response] Discarded donor data: stat_id=%u value=%u.",
                         st.stat_id(), st.stat_value());
        }
    }

    DonorPool::NoteAttemptResult(attempt.appId, attempt.poolIndex, okWithData);

    // 1. Header: force eresult = k_EResultOK.
    hdrMsg.set_eresult(1); // k_EResultOK
    const std::uint32_t newHdrSize = static_cast<std::uint32_t>(hdrMsg.ByteSizeLong());
    if (newHdrSize > outHdrCap || !hdrMsg.SerializeToArray(outHdr, static_cast<int>(outHdrCap))) {
        return kNoChange;
    }
    *outNewHdrLen = static_cast<std::int32_t>(newHdrSize);

    // 2. Body: drop the donor's progress, keeping the schema (useful).
    resp.clear_stats();

    const std::int32_t serialized = SerializeTo(resp, out, outCap, nullptr);
    if (serialized == kNoChange) return kNoChange;

    AC_LOG_INFO_ONCE(kModule,
                "[152 GetUserStats Response] Rewritten for the game: eresult=OK, %d donor stats removed, "
                "schema preserved (%u byte).",
                donorStatsCount, schemaSize);
    return serialized;
}

std::int32_t HandleRecvClientGetUserStatsResponse(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CMsgClientGetUserStatsResponse resp;
    if (!resp.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        AC_LOG_WARN(kModule, "[819 ClientGetUserStatsResponse] Body parse failed (bodyLen=%u): response unchanged.",
                    frame.bodyLen);
        return kNoChange;
    }

    steam::AppId appId = static_cast<steam::AppId>(resp.game_id());

    // AetherOnline: the response arrives with game_id 480 — resolve the real
    // appid before the pending lookup, so the response finds the attempt
    // registered by the send on redirect (real appid). The body stays
    // untouched: the game's pipe is registered as 480 and must receive
    // game_id=480.
    ResolveAetherOnlineAppId(appId, "ClientGetUserStatsResponse");

    if (!ac::luadata::HasDepot(appId)) {
        AC_LOG_DEBUG(kModule,
                    "[819 ClientGetUserStatsResponse] AppID %u not configured (HasDepot=false): "
                    "response passed through unchanged (eresult=%d).",
                    appId, resp.eresult());
        return kNoChange;
    }

    StatAttempt attempt;
    const bool wasSpoofed = DonorPool::TakePendingClientStats(appId, attempt);
    if (!wasSpoofed) {
        // WARNING: response to a request WE did not spoof (or failed to
        // correlate). This is the path to watch in the logs when investigating
        // a loss: an "OK with empty stats" response delivered to the client MAY
        // induce it to rewrite the local cache (UserGameStats_*.bin) which —
        // for unowned apps — is the only copy of progress. And it is also the
        // donor-leak door: when the response carries stats/achievements with
        // eresult OK, for a managed app it can ONLY be the donor's response
        // (two close 818s, expired 30s window, ...): full pass-through would
        // deliver the donor's achievements to the game — e.g. Cyberpunk's
        // "The Fool" unlocked on first launch (21/08/2026).
        AC_LOG_WARN(kModule,
                    "[819 ClientGetUserStatsResponse] AppID %u: response NOT correlated to one of our spoofs "
                    "(eresult=%d, stats=%d, achievement_blocks=%d, schema=%u byte fnv1a %016llx, crc_stats=%u). "
                    "Possible causes: request sent before injection, Lua config missing/reloading, "
                    "30s window expired.",
                    appId, resp.eresult(), resp.stats_size(), resp.achievement_blocks_size(),
                    resp.has_schema() ? static_cast<std::uint32_t>(resp.schema().size()) : 0u,
                    (ac::log::Enabled(LogLevel::Warn) && resp.has_schema())
                        ? static_cast<unsigned long long>(ac::hasher::Fnv1a64(resp.schema().data(), resp.schema().size()))
                        : 0ull,
                    resp.crc_stats());

        // Detail of the achievement blocks present in the response.
        if (ac::log::Enabled(LogLevel::Debug)) for (const auto& blk : resp.achievement_blocks()) {
            for (int i = 0; i < blk.unlock_time_size(); ++i) {
                AC_LOG_DEBUG(kModule,
                            "[819 ClientGetUserStatsResponse] Achievement block achievement_id=%u "
                            "unlock_time[%d]=%u (%s local).",
                            blk.achievement_id(), i, blk.unlock_time(i),
                            AchievementBackup::FormatUnixTime(blk.unlock_time(i)).c_str());
            }
        }

        // OK response WITHOUT payload: harmless (e.g. real user with no stats),
        // passes through untouched. OK response WITH payload (possible donor
        // leak) or error: normalize by emptying stats/achievements — never
        // trust data we cannot attribute to the real user with certainty.
        const bool hasPayload = resp.stats_size() > 0 || resp.achievement_blocks_size() > 0;
        if (IsOK(resp.eresult()) && !hasPayload) {
            return kNoChange;
        }
        resp.clear_stats();
        resp.clear_achievement_blocks();
        resp.clear_crc_stats();
        resp.set_eresult(1);
        const std::int32_t serialized = SerializeTo(resp, out, outCap, nullptr);
        if (serialized == kNoChange) return kNoChange;
        AC_LOG_INFO(kModule,
                    "[819 ClientGetUserStatsResponse] AppID %u: uncorrelated response %s -> normalized "
                    "with payload removed (eresult=OK, schema preserved).",
                    appId,
                    hasPayload ? "WITH payload (possible donor leak blocked)"
                               : "but eresult not OK");
        return serialized;
    }

    const bool okWithData = IsOK(resp.eresult()) && HasStatsPayload(resp);

    AC_LOG_DEBUG(kModule,
                "[819 ClientGetUserStatsResponse] AppID %u donor %llu (pool %zu): eresult=%d, schema=%u byte "
                "(fnv1a %016llx), stats=%d, achievement_blocks=%d, crc_stats=%u -> %s.",
                appId, DonorPool::DonorSteamId(attempt.poolIndex), attempt.poolIndex, resp.eresult(),
                resp.has_schema() ? static_cast<std::uint32_t>(resp.schema().size()) : 0u,
                (ac::log::Enabled(LogLevel::Debug) && resp.has_schema())
                    ? static_cast<unsigned long long>(ac::hasher::Fnv1a64(resp.schema().data(), resp.schema().size()))
                    : 0ull,
                resp.stats_size(), resp.achievement_blocks_size(), resp.crc_stats(),
                okWithData ? "donor WITH data" : "donor WITHOUT data");

    // Detail of the donor's achievement blocks about to be dropped.
    for (const auto& blk : resp.achievement_blocks()) {
        for (int i = 0; i < blk.unlock_time_size(); ++i) {
            AC_LOG_DEBUG(kModule,
                        "[819 ClientGetUserStatsResponse] Discarded donor block: achievement_id=%u "
                        "unlock_time[%d]=%u (%s local).",
                        blk.achievement_id(), i, blk.unlock_time(i),
                        AchievementBackup::FormatUnixTime(blk.unlock_time(i)).c_str());
        }
    }
    if (ac::log::Enabled(LogLevel::Debug)) for (const auto& st : resp.stats()) {
        AC_LOG_DEBUG(kModule, "[819 ClientGetUserStatsResponse] Discarded donor stat: stat_id=%u value=%u.",
                     st.stat_id(), st.stat_value());
    }

    DonorPool::NoteAttemptResult(appId, attempt.poolIndex, okWithData);

    // Total wipe of foreign unlocks, keeping the schema.
    resp.clear_stats();
    resp.clear_achievement_blocks();
    resp.clear_crc_stats();
    resp.set_eresult(1); // k_EResultOK

    const std::int32_t serialized = SerializeTo(resp, out, outCap, nullptr);
    if (serialized == kNoChange) return kNoChange;

    AC_LOG_INFO_ONCE(kModule,
                "[819 ClientGetUserStatsResponse] AppID %u rewritten for the game: eresult=OK, stats and "
                "donor achievement_blocks removed, schema preserved (%u byte). The game starts from its "
                "local cache (UserGameStats).",
                appId, resp.has_schema() ? static_cast<std::uint32_t>(resp.schema().size()) : 0u);
    return serialized;
}

// ---------------------------------------------------------------------------
// In-game Unlock Monitoring and Capture (StoreStats)
// ---------------------------------------------------------------------------

std::int32_t HandleSendStoreUserStats2(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CMsgClientStoreUserStats2 req;
    if (!req.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        AC_LOG_WARN(kModule, "[5466 StoreUserStats2] Body parse failed (bodyLen=%u): frame unchanged.",
                    frame.bodyLen);
        return kNoChange;
    }

    steam::AppId appId = static_cast<steam::AppId>(req.game_id());

    // AetherOnline: the request may arrive masked as 480 (see
    // ResolveAetherOnlineAppId) — rewrite the frame before the Lua-managed
    // gate, so in-game unlocks are attributed to the real game and the
    // SteamID rewrite (below) takes effect.
    if (ResolveAetherOnlineAppId(appId, "StoreUserStats2")) {
        req.set_game_id(appId);
    }

    if (!ac::luadata::HasDepot(appId)) {
        AC_LOG_DEBUG(kModule, "[5466 StoreUserStats2] AppID %u not configured (HasDepot=false): frame unchanged.", appId);
        return kNoChange;
    }

    const std::uint64_t originalSettor = req.settor_steam_id();
    const std::uint64_t originalSettee = req.settee_steam_id();
    std::uint64_t realSteamId = CachedActiveSteamId64();

    AC_LOG_DEBUG(kModule,
                "[5466 StoreUserStats2] AppID %u: outgoing commit with %d achievements, %d stats, "
                "crc_stats=%llu, schema_local_version=%d, settor=%llu, settee=%llu, active account=%llu.",
                appId, req.achievement_blocks_size(), req.stats_size(),
                static_cast<unsigned long long>(req.crc_stats()), req.schema_local_version(),
                static_cast<unsigned long long>(originalSettor),
                static_cast<unsigned long long>(originalSettee),
                static_cast<unsigned long long>(realSteamId));

    // Empty commit: the game sent a Store with neither achievements nor stats.
    // Not an error per se, but telling it apart in the logs helps understand
    // whether a missed save is the game's or the pipeline's fault.
    if (req.achievement_blocks_size() == 0 && req.stats_size() == 0) {
        AC_LOG_DEBUG(kModule, "[5466 StoreUserStats2] AppID %u: empty packet (0 achievements, 0 stats).",
                     appId);
    }

    // Interception of game-committed achievements.
    // Every unlock is: (1) logged with a readable timestamp, (2) saved to the
    // per-(account, appid) backup — JSON snapshot + copy of Steam's .bins —
    // so a later cache loss stays recoverable.
    const std::uint64_t mirrorAccount = realSteamId != 0 ? realSteamId
                                                         : (originalSettee != 0 ? originalSettee : originalSettor);
    AchievementBackup::TouchSession(appId, mirrorAccount);

    // Many games (e.g. Spider-Man Remastered) commit achievements NOT as
    // achievement_blocks but as STAT bitfields: stat_id = the schema bucket,
    // value = 32-bit bitfield of unlocked achievements. Real example (08/21):
    // stat_id=0 value=0x40000000 = bit 30 bucket 0 = "Wing It". We track the
    // bitfields in memory, log NEW bits (above bit 7, to avoid mistaking them
    // for slowly-growing counters) and save them into the JSON snapshot as
    // real achievements.
    {
        std::lock_guard<std::mutex> lock(g_statBitsMutex);
        auto& known = g_statBits[appId];
        std::vector<std::pair<std::uint32_t, std::uint32_t>> stats;
        stats.reserve(static_cast<std::size_t>(req.stats_size()));
        for (const auto& st : req.stats()) {
            stats.emplace_back(st.stat_id(), st.stat_value());
            const std::uint32_t value = st.stat_value();
            const bool firstSighting = known.find(st.stat_id()) == known.end();
            std::uint32_t old = 0;
            if (!firstSighting) old = known[st.stat_id()];
            const std::uint32_t newBits = (value & ~old) & 0xFFFFFF00u;   // bits >= 8 only
            if (newBits != 0) {
                // Baseline heuristic: the FIRST sighting of a stat with many new
                // bits is almost always accumulated state the game recommits at
                // startup (e.g. 15 already-unlocked achievements), not 15
                // simultaneous unlocks. We record it as baseline without
                // spamming logs/JSON (real dates stay in the backup .bins).
                const bool looksAccumulated = firstSighting && std::popcount(newBits) > 3;
                if (looksAccumulated) {
                    AC_LOG_INFO(kModule,
                                "[5466 StoreUserStats2] AppID %u stat_id=%u: baseline bitfield captured "
                                "(0x%08x, %u bit(s) set) — not an unlock wave.",
                                appId, st.stat_id(), value, std::popcount(value));
                } else {
                    for (std::uint32_t bit = 8; bit < 32; ++bit) {
                        if (newBits & (1u << bit)) {
                            const std::uint32_t achievementId = st.stat_id() * 32u + bit;
                            const std::uint32_t now = static_cast<std::uint32_t>(std::time(nullptr));
                            AC_LOG_INFO(kModule,
                                        "*** ACHIEVEMENT UNLOCKED (bitfield) *** AppID %u stat_id=%u "
                                        "0x%08x -> 0x%08x, new bit %u (achievement id %u)",
                                        appId, st.stat_id(), old, value, bit, achievementId);
                            AchievementBackup::RecordUnlock(appId, mirrorAccount, achievementId, now);
                        }
                    }
                }
            }
            known[st.stat_id()] = value;
        }
        if (!stats.empty()) {
            AchievementBackup::RecordStats(appId, mirrorAccount, stats);
        }
    }

    for (const auto& ach : req.achievement_blocks()) {
        const std::uint32_t unlockTime = ach.unlock_time();
        AC_LOG_INFO(kModule,
                    "*** ACHIEVEMENT UNLOCKED *** AppID %u achievement_id=%u unlock_time=%u (%s local) "
                    "[bucket=%u bit=%u per id=bucket*32+bit convention] -> JSON snapshot + .bin backup",
                    appId, ach.achievement_id(), unlockTime, AchievementBackup::FormatUnixTime(unlockTime).c_str(),
                    ach.achievement_id() / 32u, ach.achievement_id() % 32u);

        AchievementBackup::RecordUnlock(appId, mirrorAccount, ach.achievement_id(), unlockTime);
    }

    // Detail of the numeric stats sent by the game (for fine-grained debug:
    // counters, times, etc.). Loop skipped when Debug is filtered.
    if (ac::log::Enabled(LogLevel::Debug)) for (const auto& st : req.stats()) {
        AC_LOG_DEBUG(kModule, "[5466 StoreUserStats2] AppID %u: stat_id=%u value=%u.",
                     appId, st.stat_id(), st.stat_value());
    }

    // STEAMID MISMATCH BUG RESOLUTION:
    // If the game believes the user is the spoofed spoofing SteamID, it will try
    // to save stats for that ID, but Steam's Connection Manager will refuse the
    // write because the logged-in session belongs to the real user. We rewrite
    // settor_steam_id and settee_steam_id with the user's real active SteamID.
    if (realSteamId != 0) {
        if (originalSettor != realSteamId || originalSettee != realSteamId) {
            AC_LOG_INFO(kModule,
                        "[5466 StoreUserStats2] AppID %u: rewriting SteamID -> settor %llu -> %llu, "
                        "settee %llu -> %llu (active account).",
                        appId, static_cast<unsigned long long>(originalSettor),
                        static_cast<unsigned long long>(realSteamId),
                        static_cast<unsigned long long>(originalSettee),
                        static_cast<unsigned long long>(realSteamId));
        }
        req.set_settor_steam_id(realSteamId);
        req.set_settee_steam_id(realSteamId);

        // The module's single serialization queue (P9): same path as every
        // other handler, automatic WARN on oversized frames.
        const std::int32_t serialized = SerializeTo(req, out, outCap, "[5466 StoreUserStats2]");
        if (serialized != kNoChange) {
            AC_LOG_INFO_ONCE(kModule,
                        "[5466 StoreUserStats2] AppID %u forwarded to the server with real SteamID %llu: "
                        "NOTE: if the server rejects it (game not owned), the achievement remains valid only "
                        "in the Steam local cache (PendingChanges) and in our backup.",
                        appId, static_cast<unsigned long long>(realSteamId));
            return serialized;
        }
    } else {
        AC_LOG_WARN(kModule,
                    "[5466 StoreUserStats2] AppID %u: active SteamID unresolvable, frame unchanged "
                    "(settor=%llu, settee=%llu).",
                    appId, static_cast<unsigned long long>(originalSettor),
                    static_cast<unsigned long long>(originalSettee));
    }

    return kNoChange;
}

void Shutdown() {
    // Backup teardown (queue drain + final .bin copies + worker join) lives in
    // the dedicated module: here we only delegate it.
    AchievementBackup::FlushOnShutdown();
}

} // namespace ac::hooks::AchievementModule
