#include "pch.h"
#include "hooks/wire/AchievementModule.h"

#include <atomic>
#include <chrono>
#include <deque>
#include <mutex>
#include <string>
#include <unordered_map>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "credentials/SteamId.h"
#include "scripting/LuaData.h"
#include "steam_messages.pb.h"

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
// ============================================================================

namespace ac::hooks::AchievementModule {
namespace {

constexpr const char* kModule = "Wire.Achievement";
constexpr std::int32_t kNoChange = -1;
constexpr std::size_t kPoolCount = 15;

// 15 SteamID64 ereditati da LumaCore per il pool di fallback (byte-identical).
constexpr std::uint64_t kLumaCoreStatSteamIdPool[kPoolCount] = {
    76561198017975643ULL,
    76561198001678750ULL,
    76561198355953202ULL,
    76561197979911851ULL,
    76561198040673812ULL,
    76561198367471798ULL,
    76561198028125071ULL,
    76561198012616627ULL,
    76561197971398453ULL,
    76561197977849691ULL,
    76561198019373005ULL,
    76561198155124847ULL,
    76561198063534772ULL,
    76561198072711049ULL,
    76561198028121353ULL,
};

// LumaCore's default/primary stat SteamID (kDefaultStatSteamId) is the LAST
// entry of the pool: 76561198028121353 == pool[14]. LumaCore's
// DefaultPoolIndex() searches the pool for this ID and tries it FIRST on the
// very first request for an app. Aether previously started at pool[0], so for
// a game owned only by that donor (e.g. Endacopia) it would cycle 0,1,2,... and
// the game would give up long before ever reaching index 14. We match LumaCore
// exactly by making pool[14] the starting point for every new app.
constexpr std::size_t kDefaultPoolIndex = 14;

using Clock = std::chrono::steady_clock;

// ---------------------------------------------------------------------------
// Per-app donor pool state (LumaCore-style learning).
// ---------------------------------------------------------------------------
struct PoolEntry {
    std::size_t next = kDefaultPoolIndex;   // LumaCore starts at the default donor
    std::size_t preferred = 0;
    bool hasPreferred = false;
};

std::mutex g_poolMutex;
std::unordered_map<steam::AppId, PoolEntry> g_pool;

std::size_t PickPoolIndex(steam::AppId appId) {
    std::lock_guard<std::mutex> lock(g_poolMutex);
    auto& e = g_pool[appId];
    return e.hasPreferred ? e.preferred : e.next;
}

// okWithData = the donor actually returned useful schema/stats/achievements.
void NoteAttemptResult(steam::AppId appId, std::size_t index, bool okWithData) {
    std::lock_guard<std::mutex> lock(g_poolMutex);
    auto& e = g_pool[appId];
    if (okWithData) {
        e.preferred = index;
        e.hasPreferred = true;
        e.next = index;
        AC_LOG_INFO(kModule, "Pool AppID %u: preferito indice %zu (donor con dati).", appId, index);
        return;
    }
    if (e.hasPreferred && e.preferred == index) {
        e.hasPreferred = false;
        e.preferred = 0;
    }
    e.next = (index + 1) % kPoolCount;
    AC_LOG_DEBUG(kModule, "Pool AppID %u: avanzo indice %zu -> %zu (donor senza dati).", appId, index, e.next);
}

// ---------------------------------------------------------------------------
// Send->recv correlation so the recv path knows which donor index was used.
// ---------------------------------------------------------------------------
struct StatAttempt {
    steam::AppId appId = 0;
    std::size_t poolIndex = 0;
    std::uint64_t sequence = 0;
    Clock::time_point seen{};
};

constexpr auto kAttemptWindow = std::chrono::seconds(15);
constexpr std::size_t kAttemptCap = 24;

std::mutex g_attemptMutex;
std::unordered_map<std::uint64_t, StatAttempt> g_jobIdToAttempt;   // jobid_source -> attempt
std::deque<StatAttempt> g_recentAttempts;
std::uint64_t g_nextSequence = 1;

void PruneAttemptsLocked(Clock::time_point now) {
    std::erase_if(g_jobIdToAttempt, [&now](const auto& e) {
        return now - e.second.seen > std::chrono::seconds(30);
    });
    for (auto it = g_recentAttempts.begin(); it != g_recentAttempts.end();) {
        if (now - it->seen > kAttemptWindow) it = g_recentAttempts.erase(it);
        else ++it;
    }
    while (g_recentAttempts.size() > kAttemptCap) g_recentAttempts.pop_front();
}

void RecordAttempt(StatAttempt a, bool hasJobId, std::uint64_t jobId) {
    auto now = Clock::now();
    a.seen = now;
    std::lock_guard<std::mutex> lock(g_attemptMutex);
    PruneAttemptsLocked(now);
    a.sequence = g_nextSequence++;
    if (hasJobId) g_jobIdToAttempt[jobId] = a;
    g_recentAttempts.push_back(a);
}

// Resolves the attempt that a GetUserStats response belongs to.
bool ResolveAttempt(const CMsgProtoBufHeader& hdr, StatAttempt& out) {
    auto now = Clock::now();
    std::lock_guard<std::mutex> lock(g_attemptMutex);
    PruneAttemptsLocked(now);

    if (hdr.has_jobid_target()) {
        auto it = g_jobIdToAttempt.find(hdr.jobid_target());
        if (it != g_jobIdToAttempt.end()) {
            out = it->second;
            g_jobIdToAttempt.erase(it);
            return true;
        }
    }
    // Fallback: a single recent in-flight request for this pipe.
    StatAttempt cand;
    std::size_t n = 0;
    for (const auto& a : g_recentAttempts) {
        if (now - a.seen <= kAttemptWindow) { cand = a; ++n; }
    }
    if (n == 1) {
        out = cand;
        for (auto it = g_recentAttempts.begin(); it != g_recentAttempts.end();) {
            if (it->sequence == cand.sequence) it = g_recentAttempts.erase(it);
            else ++it;
        }
        return true;
    }
    return false;
}

// ---------------------------------------------------------------------------
// Pending spoof correlation for ClientGetUserStats (818) -> response (819).
// ---------------------------------------------------------------------------
constexpr auto kPendingWindow = std::chrono::seconds(30);
std::mutex g_pendingMutex;
std::unordered_map<steam::AppId, StatAttempt> g_pendingClientStats;

bool TakePendingClientStats(steam::AppId appId, StatAttempt& out) {
    auto now = Clock::now();
    std::lock_guard<std::mutex> lock(g_pendingMutex);
    std::erase_if(g_pendingClientStats, [&now](const auto& e) {
        return now - e.second.seen > kPendingWindow;
    });
    auto it = g_pendingClientStats.find(appId);
    if (it == g_pendingClientStats.end()) return false;
    out = it->second;
    g_pendingClientStats.erase(it);
    return true;
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
// OnlineFix session resolution (single source of truth for this module).
//
// With the "-onlinefix" launch flag the game process is masked as Spacewar
// (AppID 480): every UserStats frame it sends carries appid/game_id == 480,
// and the Lua-managed gate below (HasDepot) would reject it because depot
// keys are registered under the REAL app id. Rewriting the frame to the real
// app id restores the donor-spoofing pipeline for OnlineFix sessions.
//
// The real app id is captured at spawn time by OnlineFixHooks::h_SpawnProcess
// into g_state.onlineFixRealAppId. When no session is active (or the id is
// already the real one) the frame is left untouched.
//
// Returns true when 'appId' was rewritten, so callers can mirror the new id
// back into their protobuf request (the 819 response path has no field to
// mirror — it only uses the resolved id for correlation/gating).
bool ResolveOnlineFixAppId(steam::AppId& appId, const char* flow) {
    const steam::AppId real = g_state.onlineFixRealAppId.load();
    if (appId == constants::kSpacewarAppId && real != 0 && real != constants::kSpacewarAppId) {
        AC_LOG_INFO(kModule, "OnlineFix: %s AppID %u -> %u.", flow, appId, real);
        appId = real;
        return true;
    }
    return false;
}

} // namespace

// ---------------------------------------------------------------------------
// Intercettazione e Spoofing delle Richieste in Uscita (HandleSend)
// ---------------------------------------------------------------------------

std::int32_t HandleSendGetUserStats(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CPlayer_GetUserStats_Request req;
    if (!req.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        return kNoChange;
    }

    steam::AppId appId = req.appid();

    // OnlineFix: la richiesta può arrivare mascherata come 480 (vedi
    // ResolveOnlineFixAppId) — riscrivi la frame prima del gate Lua-managed.
    if (ResolveOnlineFixAppId(appId, "GetUserStats")) {
        req.set_appid(appId);
    }

    if (!ac::luadata::HasDepot(appId)) {
        return kNoChange;
    }

    // LumaCore behaviour: clear any local schema and spoof anyway. The old
    // code bailed out here when the request already carried a sha_schema,
    // which is exactly why achievements failed for games that send a cached
    // schema (e.g. Endacopia / AppID 2684630).
    req.clear_sha_schema();

    const std::size_t poolIndex = PickPoolIndex(appId);
    const std::uint64_t donorId = kLumaCoreStatSteamIdPool[poolIndex];

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
    RecordAttempt(attempt, hasJobId, jobId);

    req.set_steamid(donorId);

    const std::uint32_t size = static_cast<std::uint32_t>(req.ByteSizeLong());
    if (size > outCap || !req.SerializeToArray(out, static_cast<int>(outCap))) {
        return kNoChange;
    }

    AC_LOG_INFO(kModule, "Spoofing GetUserStats (eMsg 151) per AppID %u con DonorID %llu (pool %zu)",
                appId, donorId, poolIndex);
    return static_cast<std::int32_t>(size);
}

std::int32_t HandleSendClientGetUserStats(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CMsgClientGetUserStats req;
    if (!req.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        return kNoChange;
    }

    steam::AppId appId = static_cast<steam::AppId>(req.game_id());

    // OnlineFix: la richiesta può arrivare mascherata come 480 (vedi
    // ResolveOnlineFixAppId) — riscrivi la frame prima del gate Lua-managed.
    if (ResolveOnlineFixAppId(appId, "ClientGetUserStats")) {
        req.set_game_id(appId);
    }

    if (!ac::luadata::HasDepot(appId)) {
        return kNoChange;
    }

    // Forza la richiesta dello schema azzerando la versione locale (ottiene lo
    // schema aggiornato) e la crc locale (come LumaCore).
    req.clear_crc_stats();
    req.set_schema_local_version(-1);

    const std::size_t poolIndex = PickPoolIndex(appId);
    const std::uint64_t donorId = kLumaCoreStatSteamIdPool[poolIndex];
    req.set_steam_id_for_user(donorId);

    StatAttempt attempt;
    attempt.appId = appId;
    attempt.poolIndex = poolIndex;
    {
        std::lock_guard<std::mutex> lock(g_pendingMutex);
        auto now = Clock::now();
        std::erase_if(g_pendingClientStats, [&now](const auto& e) {
            return now - e.second.seen > kPendingWindow;
        });
        g_pendingClientStats[appId] = attempt;
    }

    const std::uint32_t size = static_cast<std::uint32_t>(req.ByteSizeLong());
    if (size > outCap || !req.SerializeToArray(out, static_cast<int>(outCap))) {
        return kNoChange;
    }

    AC_LOG_INFO(kModule, "Spoofing ClientGetUserStats (eMsg 818) per AppID %u con DonorID %llu (pool %zu)",
                appId, donorId, poolIndex);
    return static_cast<std::int32_t>(size);
}

// ---------------------------------------------------------------------------
// Normalizzazione delle Risposte in Entrata (HandleRecv)
// ---------------------------------------------------------------------------

std::int32_t HandleRecvGetUserStatsResponse(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap,
                                            std::uint8_t* outHdr, std::uint32_t outHdrCap, std::int32_t* outNewHdrLen) {
    // Resolve which (spoofed) request this response belongs to.
    CMsgProtoBufHeader hdrMsg;
    if (!hdrMsg.ParseFromArray(frame.header, static_cast<int>(frame.headerLen))) {
        return kNoChange;
    }
    StatAttempt attempt;
    if (!ResolveAttempt(hdrMsg, attempt) || !ac::luadata::HasDepot(attempt.appId)) {
        return kNoChange;
    }

    // Learn which donor actually returned useful data.
    CPlayer_GetUserStats_Response resp;
    if (!resp.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        return kNoChange;
    }
    const std::int32_t originalResult = hdrMsg.has_eresult() ? hdrMsg.eresult() : -1;
    const bool okWithData = IsOK(originalResult) && HasStatsPayload(resp);
    NoteAttemptResult(attempt.appId, attempt.poolIndex, okWithData);

    // 1. Header: forza eresult = k_EResultOK.
    hdrMsg.set_eresult(1); // k_EResultOK
    const std::uint32_t newHdrSize = static_cast<std::uint32_t>(hdrMsg.ByteSizeLong());
    if (newHdrSize > outHdrCap || !hdrMsg.SerializeToArray(outHdr, static_cast<int>(outHdrCap))) {
        return kNoChange;
    }
    *outNewHdrLen = static_cast<std::int32_t>(newHdrSize);

    // 2. Body: rimuovi i progressi del donatore, tenendo lo schema (utile).
    resp.clear_stats();

    const std::uint32_t size = static_cast<std::uint32_t>(resp.ByteSizeLong());
    if (size > outCap || !resp.SerializeToArray(out, static_cast<int>(outCap))) {
        return kNoChange;
    }

    AC_LOG_INFO(kModule, "Risposta GetUserStats riscritta (eresult=OK, stats ripulite, donor %s).",
                okWithData ? "con schema" : "senza schema");
    return static_cast<std::int32_t>(size);
}

std::int32_t HandleRecvClientGetUserStatsResponse(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CMsgClientGetUserStatsResponse resp;
    if (!resp.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        return kNoChange;
    }

    steam::AppId appId = static_cast<steam::AppId>(resp.game_id());

    // OnlineFix: la risposta arriva con game_id 480 — risolvi l'appid reale
    // prima del lookup pending, così il response trova l'attempt registrato
    // dal send sul redirect (appid reale). Il body resta invariato: il pipe
    // del gioco è registrato come 480 e deve ricevere game_id=480.
    ResolveOnlineFixAppId(appId, "ClientGetUserStatsResponse");

    if (!ac::luadata::HasDepot(appId)) {
        return kNoChange;
    }

    StatAttempt attempt;
    const bool wasSpoofed = TakePendingClientStats(appId, attempt);
    if (!wasSpoofed) {
        // Non-spoofed by us: leave untouched if OK, otherwise normalize to OK.
        if (IsOK(resp.eresult())) {
            return kNoChange;
        }
        resp.clear_stats();
        resp.clear_achievement_blocks();
        resp.clear_crc_stats();
        resp.set_eresult(1);
        const std::uint32_t size = static_cast<std::uint32_t>(resp.ByteSizeLong());
        if (size > outCap || !resp.SerializeToArray(out, static_cast<int>(outCap))) {
            return kNoChange;
        }
        AC_LOG_INFO(kModule, "Risposta ClientGetUserStats (eMsg 819) per AppID %u: non-spoofed ma eresult non-OK, normalizzata.",
                    appId);
        return static_cast<std::int32_t>(size);
    }

    const bool okWithData = IsOK(resp.eresult()) && HasStatsPayload(resp);
    NoteAttemptResult(appId, attempt.poolIndex, okWithData);

    // Svuotamento totale degli sblocchi estranei, mantenendo lo schema.
    resp.clear_stats();
    resp.clear_achievement_blocks();
    resp.clear_crc_stats();
    resp.set_eresult(1); // k_EResultOK

    const std::uint32_t size = static_cast<std::uint32_t>(resp.ByteSizeLong());
    if (size > outCap || !resp.SerializeToArray(out, static_cast<int>(outCap))) {
        return kNoChange;
    }

    AC_LOG_INFO(kModule, "Risposta ClientGetUserStatsResponse (eMsg 819) per AppID %u riscritta (stats rimosse, donor %s).",
                appId, okWithData ? "con schema" : "senza schema");
    return static_cast<std::int32_t>(size);
}

// ---------------------------------------------------------------------------
// Monitoraggio e Cattura degli Sblocchi in-game (StoreStats)
// ---------------------------------------------------------------------------

std::int32_t HandleSendStoreUserStats2(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CMsgClientStoreUserStats2 req;
    if (!req.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        return kNoChange;
    }

    steam::AppId appId = static_cast<steam::AppId>(req.game_id());

    // OnlineFix: la richiesta può arrivare mascherata come 480 (vedi
    // ResolveOnlineFixAppId) — riscrivi la frame prima del gate Lua-managed,
    // così gli sblocchi in-game vengono attribuiti al gioco reale e la
    // riscrittura dello SteamID (sotto) ha effetto.
    if (ResolveOnlineFixAppId(appId, "StoreUserStats2")) {
        req.set_game_id(appId);
    }

    if (!ac::luadata::HasDepot(appId)) {
        return kNoChange;
    }

    // Intercettazione degli achievement committati dal gioco
    for (const auto& ach : req.achievement_blocks()) {
        AC_LOG_INFO(kModule, "NOTIFICA EVENTO: Sbloccato Achievement %u per AppID %u!", ach.achievement_id(), appId);
    }

    // RISOLUZIONE BUG DI MISMATCH STEAMID:
    // Se il gioco crede che l'utente sia lo SteamID fittizio di spoofing, tenterà
    // di salvare le stats per quell'ID, ma il Connection Manager di Steam rifiuterà
    // la scrittura perché la sessione loggata appartiene al vero utente. Riscriviamo
    // settor_steam_id e settee_steam_id con il vero SteamID attivo dell'utente.
    std::uint64_t realSteamId = ac::steamid::GetActiveSteamId64();
    if (realSteamId != 0) {
        req.set_settor_steam_id(realSteamId);
        req.set_settee_steam_id(realSteamId);

        const std::uint32_t size = static_cast<std::uint32_t>(req.ByteSizeLong());
        if (size <= outCap && req.SerializeToArray(out, static_cast<int>(outCap))) {
            AC_LOG_INFO(kModule, "Modificato StoreUserStats2 per AppID %u: impostato SteamID reale %llu", appId, realSteamId);
            return static_cast<std::int32_t>(size);
        }
    }

    return kNoChange;
}

void Shutdown() {
    // No background worker anymore (OpenSteamTool API removed); the pool and
    // attempt maps are small and will be reclaimed on unload. Nothing to join.
}

} // namespace ac::hooks::AchievementModule
