#include "pch.h"
#include "hooks/wire/AchievementModule.h"

#include <atomic>
#include <bit>
#include <chrono>
#include <ctime>
#include <deque>
#include <mutex>
#include <string>
#include <unordered_map>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "credentials/SteamId.h"
#include "hooks/wire/AchievementBackup.h"
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
// LOGGING, MIRROR E BACKUP (sessione di debug agosto 2026 — perdita achievement):
//
// Il client Steam conserva gli achievement SOLO nella cache locale
// (<steam>\appcache\stats\UserGameStats_<account>_<appid>.bin): il server
// rifiuta i StoreUserStats2 degli account che non possiedono il gioco, quindi
// quella cache è l'UNICA copia dei progressi e qualunque invalidazione
// client-side è una perdita definitiva (vedi anche il campo "PendingChanges"
// nel file binario: cambi mai confermati dal server).
//
// Per questo ogni fase del flusso lascia una traccia dettagliata nel log e,
// soprattutto, ogni sblocco viene replicato nella sezione Backup di AetherDesk
// (se installata, tramite desk_path.cfg) con la stessa convenzione di nomi di
// Steam — un file per (account, appid). La persistenza vive nel modulo dedicato
// hooks/wire/AchievementBackup (separazione dei compiti): RecordUnlock() è
// asincrona, nessuna I/O su disco nel thread di rete di Steam.
//
//   <AetherData>\backup\<appid>\achievements\            [via desk_path.cfg]
//   <steam>\aethercore\backup\<appid>\achievements\      [fallback]
//       UserGameStats_<account>_<appid>.json   snapshot sblocchi (leggibile,
//                                               rigenera il .bin con il tool)
//       UserGameStats_<account>_<appid>.bin    copia della cache Steam
//       UserGameStatsSchema_<appid>.bin        copia dello schema del gioco
//
// Se la cache di Steam venisse di nuovo azzerata, dal backup si recupera tutto:
// o rimettendo i .bin al loro posto (Steam chiuso), o ricostruendo il .bin dal
// JSON con Tools/achievement_decoder.py (comando `rebuild`).
//
// Flusso tracciato (con eMsg):
//   151  Send  Player.GetUserStats (servizio)          -> donor spoofing
//   152  Recv  Player.GetUserStats response (147)      -> pulizia dati donor
//   818  Send  ClientGetUserStats                      -> donor spoofing
//   819  Recv  ClientGetUserStatsResponse              -> pulizia dati donor
//   5466 Send  ClientStoreUserStats2                   -> SBLOCCO achievement
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

// Ultimo valore noto delle stat bitfield per app (stat_id -> value), per
// rilevare i NUOVI bit a ogni commit (achievement via stats).
std::mutex g_statBitsMutex;
std::unordered_map<steam::AppId, std::unordered_map<std::uint32_t, std::uint32_t>> g_statBits;

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
        AC_LOG_INFO_ONCE(kModule, "Pool AppID %u: preferred index %zu (donor %llu has data).",
                    appId, index, kLumaCoreStatSteamIdPool[index]);
        return;
    }
    if (e.hasPreferred && e.preferred == index) {
        e.hasPreferred = false;
        e.preferred = 0;
    }
    e.next = (index + 1) % kPoolCount;
    AC_LOG_DEBUG(kModule, "Pool AppID %u: advancing index %zu -> %zu (donor has no data).", appId, index, e.next);
}

// ---------------------------------------------------------------------------
// Send->recv correlation so the recv path knows which donor index was used.
// ---------------------------------------------------------------------------
// Esito della correlazione tra una risposta e le richieste che abbiamo
// spoofato. NoMatch = risposta per una richiesta NON nostra (app posseduto o
// traffico interno di Steam): va lasciata passare intatta. Ambiguous = più
// richieste spoofate in volo e non sappiamo a quale appartiene la risposta:
// in quel caso il payload è del DONOR con quasi certezza e va ripulito
// (vedi il leak "The Fool" di Cyberpunk, 21/08/2026: il donor aveva
// l'achievement e una risposta non correlata lo ha consegnato al gioco).
enum class Correl {
    Resolved,    // attempt riempito e valido
    Ambiguous,   // più attempt in-flight sovrapposti: appId sconosciuto
    NoMatch,     // nessun attempt nostro: pass-through corretto
};

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
Correl ResolveAttempt(const CMsgProtoBufHeader& hdr, StatAttempt& out) {
    auto now = Clock::now();
    std::lock_guard<std::mutex> lock(g_attemptMutex);
    PruneAttemptsLocked(now);

    if (hdr.has_jobid_target()) {
        auto it = g_jobIdToAttempt.find(hdr.jobid_target());
        if (it != g_jobIdToAttempt.end()) {
            out = it->second;
            g_jobIdToAttempt.erase(it);
            // Correlazione riuscita: rimuovi l'attempt ANCHE dalla coda di
            // fallback, altrimenti resta come "fantasma" per kAttemptWindow e
            // rende ambiguo (n>1) il fallback della risposta successiva, che a
            // quel punto passerebbe al gioco NON riscritta (con le stats del
            // donor). Segnalato dalla revisione esterna del 21/08/2026.
            for (auto itDq = g_recentAttempts.begin(); itDq != g_recentAttempts.end();) {
                if (itDq->sequence == out.sequence) itDq = g_recentAttempts.erase(itDq);
                else ++itDq;
            }
            AC_LOG_DEBUG(kModule, "Response correlation via jobid_target %llu -> attempt AppID %u (pool %zu).",
                        static_cast<unsigned long long>(hdr.jobid_target()), out.appId, out.poolIndex);
            return Correl::Resolved;
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
        AC_LOG_DEBUG(kModule, "Response correlation without jobid: single recent attempt -> AppID %u (pool %zu).",
                    out.appId, out.poolIndex);
        return Correl::Resolved;
    }
    if (n > 1) {
        AC_LOG_WARN(kModule,
                    "Ambiguous correlation: %zu overlapping spoofed requests in flight. The response will "
                    "be stripped of the donor payload for safety (no leak to the game).",
                    n);
        return Correl::Ambiguous;
    }
    return Correl::NoMatch;
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

// Cache dello SteamID attivo: GetActiveSteamId64 legge registry/filesystem a
// ogni chiamata, ma il 5466 può arrivare molte volte al minuto. L'account non
// cambia senza un riavvio di Steam: TTL di 30s (compromesso tra freschezza e
// costo), atomici senza lock (il valore a 64 bit è atomico su x64).
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
        AC_LOG_INFO_ONCE(kModule, "OnlineFix: %s AppID %u -> %u.", flow, appId, real);
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
        AC_LOG_WARN(kModule, "[151 GetUserStats] Parse failed (bodyLen=%u): frame unchanged.", frame.bodyLen);
        return kNoChange;
    }

    steam::AppId appId = req.appid();

    // OnlineFix: la richiesta può arrivare mascherata come 480 (vedi
    // ResolveOnlineFixAppId) — riscrivi la frame prima del gate Lua-managed.
    if (ResolveOnlineFixAppId(appId, "GetUserStats")) {
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
        AC_LOG_WARN(kModule, "[151 GetUserStats] Rewritten serialization failed (size=%u, cap=%u): frame unchanged.",
                    size, outCap);
        return kNoChange;
    }

    AC_LOG_INFO_ONCE(kModule, "[151 GetUserStats] Spoofing AppID %u: steamid %llu -> donor %llu (pool %zu, jobid %s).",
                appId, static_cast<unsigned long long>(originalSteamId), donorId, poolIndex, hasJobId ? "yes" : "no");
    return static_cast<std::int32_t>(size);
}

std::int32_t HandleSendClientGetUserStats(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CMsgClientGetUserStats req;
    if (!req.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        AC_LOG_WARN(kModule, "[818 ClientGetUserStats] Parse failed (bodyLen=%u): frame unchanged.", frame.bodyLen);
        return kNoChange;
    }

    steam::AppId appId = static_cast<steam::AppId>(req.game_id());

    // OnlineFix: la richiesta può arrivare mascherata come 480 (vedi
    // ResolveOnlineFixAppId) — riscrivi la frame prima del gate Lua-managed.
    if (ResolveOnlineFixAppId(appId, "ClientGetUserStats")) {
        req.set_game_id(appId);
    }

    if (!ac::luadata::HasDepot(appId)) {
        AC_LOG_DEBUG(kModule, "[818 ClientGetUserStats] AppID %u not configured (HasDepot=false): frame unchanged.", appId);
        return kNoChange;
    }

    const std::size_t poolIndex = PickPoolIndex(appId);
    const std::uint64_t donorId = kLumaCoreStatSteamIdPool[poolIndex];
    const std::uint64_t originalUserId = req.steam_id_for_user();

    // Backup di sessione: al primo 818 dell'app copia la cache .bin esistente
    // (protegge i progressi già accumulati anche senza nuovi sblocchi).
    AchievementBackup::TouchSession(appId, originalUserId);

    AC_LOG_DEBUG(kModule,
                "[818 ClientGetUserStats] Outgoing request for AppID %u: steam_id_for_user=%llu, "
                "crc_stats=%u, schema_local_version=%d -> spoofing with donor %llu (pool %zu), "
                "crc_stats cleared and schema_local_version=-1.",
                appId, static_cast<unsigned long long>(originalUserId), req.crc_stats(),
                req.schema_local_version(), donorId, poolIndex);

    // Forza la richiesta dello schema azzerando la versione locale (ottiene lo
    // schema aggiornato) e la crc locale (come LumaCore).
    req.clear_crc_stats();
    req.set_schema_local_version(-1);

    req.set_steam_id_for_user(donorId);

    StatAttempt attempt;
    attempt.appId = appId;
    attempt.poolIndex = poolIndex;
    attempt.seen = Clock::now();   // FIX (log 21/08): senza questo il default è
                                   // epoch → il prune di TakePendingClientStats
                                   // considerava la entry già scaduta e la
                                   // cancellava: OGNI 819 risultava "non
                                   // correlata" (e con donor con dati, leak).
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
        AC_LOG_WARN(kModule, "[818 ClientGetUserStats] Rewritten serialization failed (size=%u, cap=%u): frame unchanged.",
                    size, outCap);
        return kNoChange;
    }

    AC_LOG_DEBUG(kModule, "[818 ClientGetUserStats] Spoofing AppID %u with DonorID %llu (pool %zu): rewritten and forwarded.",
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
    const Correl correl = ResolveAttempt(hdrMsg, attempt);
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

    // Risposta del DONOR non correlabile con precisione: Ambiguous (più
    // richieste spoofate in volo) oppure Resolved su un app diventato non
    // gestito dopo il send (es. MarkOwned scattato nel frattempo). In entrambi
    // i casi il payload appartiene al donor e NON deve raggiungere il gioco
    // (leak "The Fool": achievement del donor consegnati all'utente). Ripuliamo
    // stats e crc, conserviamo lo schema, header invariato.
    if (correl == Correl::Ambiguous || !ac::luadata::HasDepot(attempt.appId)) {
        const int removedStats = resp.stats_size();
        const std::uint32_t schemaBytes = resp.has_schema() ? static_cast<std::uint32_t>(resp.schema().size()) : 0u;
        resp.clear_stats();
        resp.clear_crc_stats();
        const std::uint32_t size = static_cast<std::uint32_t>(resp.ByteSizeLong());
        if (size > outCap || !resp.SerializeToArray(out, static_cast<int>(outCap))) {
            return kNoChange;
        }
        AC_LOG_WARN(kModule,
                    "[152 GetUserStats Response] Donor response %s: %d stats removed for safety, "
                    "schema preserved (%u byte). No leak to the game.",
                    correl == Correl::Ambiguous ? "not correlatable (overlapping requests)"
                                                : "for an app no longer managed",
                    removedStats, schemaBytes);
        return static_cast<std::int32_t>(size);
    }
    const std::int32_t originalResult = hdrMsg.has_eresult() ? hdrMsg.eresult() : -1;
    const bool okWithData = IsOK(originalResult) && HasStatsPayload(resp);
    const int donorStatsCount = resp.stats_size();
    const std::uint32_t schemaSize = resp.has_schema() ? static_cast<std::uint32_t>(resp.schema().size()) : 0u;

    AC_LOG_DEBUG(kModule,
                "[152 GetUserStats Response] AppID %u donor %llu (pool %zu): eresult=%d, schema=%u byte "
                "(fnv1a %016llx, sha_schema %s), stats=%d, crc_stats=%u -> donor %s.",
                attempt.appId, kLumaCoreStatSteamIdPool[attempt.poolIndex], attempt.poolIndex, originalResult,
                resp.has_schema() ? static_cast<std::uint32_t>(resp.schema().size()) : 0u,
                (ac::log::Enabled(LogLevel::Debug) && resp.has_schema())
                    ? static_cast<unsigned long long>(ac::hasher::Fnv1a64(resp.schema().data(), resp.schema().size()))
                    : 0ull,
                (resp.has_sha_schema() && !resp.sha_schema().empty()) ? "present" : "absent",
                resp.stats_size(), resp.crc_stats(), okWithData ? "WITH data" : "WITHOUT data");

    // Dettaglio dei dati del donor che stanno per essere rimossi (debug).
    // Il loop viene saltato interamente quando Debug non è attivo: il donor
    // può portare decine di stat e il parsing per il log non è gratis.
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

    AC_LOG_INFO_ONCE(kModule,
                "[152 GetUserStats Response] Rewritten for the game: eresult=OK, %d donor stats removed, "
                "schema preserved (%u byte).",
                donorStatsCount, schemaSize);
    return static_cast<std::int32_t>(size);
}

std::int32_t HandleRecvClientGetUserStatsResponse(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CMsgClientGetUserStatsResponse resp;
    if (!resp.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        AC_LOG_WARN(kModule, "[819 ClientGetUserStatsResponse] Body parse failed (bodyLen=%u): response unchanged.",
                    frame.bodyLen);
        return kNoChange;
    }

    steam::AppId appId = static_cast<steam::AppId>(resp.game_id());

    // OnlineFix: la risposta arriva con game_id 480 — risolvi l'appid reale
    // prima del lookup pending, così il response trova l'attempt registrato
    // dal send sul redirect (appid reale). Il body resta invariato: il pipe
    // del gioco è registrato come 480 e deve ricevere game_id=480.
    ResolveOnlineFixAppId(appId, "ClientGetUserStatsResponse");

    if (!ac::luadata::HasDepot(appId)) {
        AC_LOG_DEBUG(kModule,
                    "[819 ClientGetUserStatsResponse] AppID %u not configured (HasDepot=false): "
                    "response passed through unchanged (eresult=%d).",
                    appId, resp.eresult());
        return kNoChange;
    }

    StatAttempt attempt;
    const bool wasSpoofed = TakePendingClientStats(appId, attempt);
    if (!wasSpoofed) {
        // ATTENZIONE: risposta per una richiesta che NON abbiamo spoofato noi
        // (o che non siamo riusciti a correlare). È il percorso da tenere
        // d'occhio nei log quando si indaga una perdita: una risposta "OK con
        // stats vuote" consegnata al client PUO' indurlo a riscrivere la
        // cache locale (UserGameStats_*.bin) che — per gli app non posseduti —
        // è l'unica copia dei progressi. Ed è anche la porta del leak donor:
        // se la risposta porta stats/achievement con eresult OK, per un app
        // gestito può trattarsi SOLO della risposta del donor (due 818
        // ravvicinati, finestra 30s scaduta, ...): il pass-through integrale
        // consegnerebbe al gioco gli achievement del donor — es. "The Fool"
        // di Cyberpunk sbloccato al primo avvio (21/08/2026).
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

        // Dettaglio dei blocchi achievement presenti nella risposta.
        if (ac::log::Enabled(LogLevel::Debug)) for (const auto& blk : resp.achievement_blocks()) {
            for (int i = 0; i < blk.unlock_time_size(); ++i) {
                AC_LOG_DEBUG(kModule,
                            "[819 ClientGetUserStatsResponse] Achievement block achievement_id=%u "
                            "unlock_time[%d]=%u (%s local).",
                            blk.achievement_id(), i, blk.unlock_time(i),
                            AchievementBackup::FormatUnixTime(blk.unlock_time(i)).c_str());
            }
        }

        // Risposta OK SENZA payload: innocua (es. utente reale senza stats),
        // passa intatta. Risposta OK CON payload (possibile donor leak) o
        // errore: normalizziamo svuotando stats/achievement — mai fidarsi di
        // dati che non possiamo attribuire con certezza al vero utente.
        const bool hasPayload = resp.stats_size() > 0 || resp.achievement_blocks_size() > 0;
        if (IsOK(resp.eresult()) && !hasPayload) {
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
        AC_LOG_INFO(kModule,
                    "[819 ClientGetUserStatsResponse] AppID %u: uncorrelated response %s -> normalized "
                    "with payload removed (eresult=OK, schema preserved).",
                    appId,
                    hasPayload ? "WITH payload (possible donor leak blocked)"
                               : "but eresult not OK");
        return static_cast<std::int32_t>(size);
    }

    const bool okWithData = IsOK(resp.eresult()) && HasStatsPayload(resp);

    AC_LOG_DEBUG(kModule,
                "[819 ClientGetUserStatsResponse] AppID %u donor %llu (pool %zu): eresult=%d, schema=%u byte "
                "(fnv1a %016llx), stats=%d, achievement_blocks=%d, crc_stats=%u -> %s.",
                appId, kLumaCoreStatSteamIdPool[attempt.poolIndex], attempt.poolIndex, resp.eresult(),
                resp.has_schema() ? static_cast<std::uint32_t>(resp.schema().size()) : 0u,
                (ac::log::Enabled(LogLevel::Debug) && resp.has_schema())
                    ? static_cast<unsigned long long>(ac::hasher::Fnv1a64(resp.schema().data(), resp.schema().size()))
                    : 0ull,
                resp.stats_size(), resp.achievement_blocks_size(), resp.crc_stats(),
                okWithData ? "donor WITH data" : "donor WITHOUT data");

    // Dettaglio dei blocchi achievement del donor che stanno per essere rimossi.
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

    AC_LOG_INFO_ONCE(kModule,
                "[819 ClientGetUserStatsResponse] AppID %u rewritten for the game: eresult=OK, stats and "
                "donor achievement_blocks removed, schema preserved (%u byte). The game starts from its "
                "local cache (UserGameStats).",
                appId, resp.has_schema() ? static_cast<std::uint32_t>(resp.schema().size()) : 0u);
    return static_cast<std::int32_t>(size);
}

// ---------------------------------------------------------------------------
// Monitoraggio e Cattura degli Sblocchi in-game (StoreStats)
// ---------------------------------------------------------------------------

std::int32_t HandleSendStoreUserStats2(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CMsgClientStoreUserStats2 req;
    if (!req.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        AC_LOG_WARN(kModule, "[5466 StoreUserStats2] Body parse failed (bodyLen=%u): frame unchanged.",
                    frame.bodyLen);
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

    // Commit vuoto: il gioco ha inviato uno Store senza ne' achievement ne'
    // stat. Non e' un errore di per se', ma distinguerlo nei log aiuta a
    // capire se un mancato salvataggio dipende dal gioco o dal flusso.
    if (req.achievement_blocks_size() == 0 && req.stats_size() == 0) {
        AC_LOG_DEBUG(kModule, "[5466 StoreUserStats2] AppID %u: empty packet (0 achievements, 0 stats).",
                     appId);
    }

    // Intercettazione degli achievement committati dal gioco.
    // Ogni sblocco viene: (1) loggato con timestamp leggibile, (2) salvato nel
    // backup per (account, appid) — snapshot JSON + copia dei .bin di Steam —
    // così una successiva perdita della cache resta recuperabile.
    const std::uint64_t mirrorAccount = realSteamId != 0 ? realSteamId
                                                         : (originalSettee != 0 ? originalSettee : originalSettor);
    AchievementBackup::TouchSession(appId, mirrorAccount);

    // Molti giochi (es. Spider-Man Remastered) committano gli achievement NON
    // come achievement_blocks ma come STAT bitfield: stat_id = bucket dello
    // schema, value = bitfield 32 bit degli achievement sbloccati. Esempio
    // reale (21/08): stat_id=0 value=0x40000000 = bit 30 bucket 0 = "Wing It".
    // Tracciamo i bitfield in memoria, logghiamo i NUOVI bit (sopra il bit 7,
    // per non confonderli con contatori che crescono di poco) e li salviamo
    // nello snapshot JSON come achievement veri.
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
            const std::uint32_t newBits = (value & ~old) & 0xFFFFFF00u;   // solo bit >= 8
            if (newBits != 0) {
                // Euristica baseline: il PRIMO avvistamento di una stat con molti
                // bit nuovi è quasi sempre lo stato accumulato che il gioco
                // ricommita all'avvio (es. 15 achievement gia' sbloccati), non
                // 15 sblocchi simultanei. Lo registriamo come baseline senza
                // spammare log/JSON (le date vere restano nei .bin del backup).
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

    // Dettaglio delle stat numeriche inviate dal gioco (per debug finemente
    // granulari: contatori, tempi, ecc.). Loop saltato quando Debug è filtrato.
    if (ac::log::Enabled(LogLevel::Debug)) for (const auto& st : req.stats()) {
        AC_LOG_DEBUG(kModule, "[5466 StoreUserStats2] AppID %u: stat_id=%u value=%u.",
                     appId, st.stat_id(), st.stat_value());
    }

    // RISOLUZIONE BUG DI MISMATCH STEAMID:
    // Se il gioco crede che l'utente sia lo SteamID fittizio di spoofing, tenterà
    // di salvare le stats per quell'ID, ma il Connection Manager di Steam rifiuterà
    // la scrittura perché la sessione loggata appartiene al vero utente. Riscriviamo
    // settor_steam_id e settee_steam_id con il vero SteamID attivo dell'utente.
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

        const std::uint32_t size = static_cast<std::uint32_t>(req.ByteSizeLong());
        if (size <= outCap && req.SerializeToArray(out, static_cast<int>(outCap))) {
            AC_LOG_INFO_ONCE(kModule,
                        "[5466 StoreUserStats2] AppID %u forwarded to the server with real SteamID %llu: "
                        "NOTE: if the server rejects it (game not owned), the achievement remains valid only "
                        "in the Steam local cache (PendingChanges) and in our backup.",
                        appId, static_cast<unsigned long long>(realSteamId));
            return static_cast<std::int32_t>(size);
        }
        AC_LOG_WARN(kModule, "[5466 StoreUserStats2] AppID %u: rewritten serialization failed (size=%u, cap=%u).",
                    appId, size, outCap);
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
    // Il teardown del backup (scarico coda + ultime copie .bin + join del
    // worker) vive nel modulo dedicato: qui ci limitiamo a delegarlo.
    AchievementBackup::FlushOnShutdown();
}

} // namespace ac::hooks::AchievementModule
