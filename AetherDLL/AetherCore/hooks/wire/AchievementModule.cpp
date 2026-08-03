#include "pch.h"
#include "hooks/wire/AchievementModule.h"

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <deque>
#include <mutex>
#include <shared_mutex>
#include <string>
#include <thread>
#include <unordered_set>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "credentials/SteamId.h"
#include "network/RuntimeHttp.h"
#include "steam_messages.pb.h"

namespace ac::hooks::AchievementModule {
namespace {

constexpr const char* kModule = "Wire.Achievement";
constexpr std::int32_t kNoChange = -1;

// 15 SteamID64 ereditati da LumaCore per il pool di fallback
constexpr std::uint64_t kLumaCoreStatSteamIdPool[15] = {
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
    76561198028121353ULL, // Fallback finale (usato anche da OpenSteamTool)
};

// ---------------------------------------------------------------------------
// Background donor-ID worker (A1).
//
// Resolving the donor SteamID (OpenSteamTool API) is HTTP I/O and must NEVER
// run on Steam's network thread (PacketRouter). This worker owns the HTTP call;
// the wire path only reads the cache or (on miss) schedules a background
// resolve and immediately falls back to the Luma pool. Single-flight: one HTTP
// GET per app id at a time; concurrent requests for the same app are coalesced.
// ---------------------------------------------------------------------------
std::mutex s_donorMutex;
std::condition_variable s_donorCv;
std::deque<steam::AppId> s_donorQueue;
std::unordered_set<steam::AppId> s_donorInflight;
std::thread s_donorWorker;
std::atomic<bool> s_donorStop{false};
std::atomic<bool> s_donorStarted{false};
std::atomic<std::uint64_t> s_donorResolvesDone{0};
std::atomic<std::uint64_t> s_donorResolvesFailed{0};

void DonorWorkerMain() {
    for (;;) {
        steam::AppId appId = 0;
        {
            std::unique_lock<std::mutex> lock(s_donorMutex);
            s_donorCv.wait(lock, [] {
                return s_donorStop.load(std::memory_order_relaxed) || !s_donorQueue.empty();
            });
            if (s_donorStop.load(std::memory_order_relaxed)) return;
            appId = s_donorQueue.front();
            s_donorQueue.pop_front();
        }

        const std::string url = "https://stats.opensteamtool.com/" + std::to_string(appId);
        AC_LOG_INFO(kModule, "Donor resolve (background) AppID %u...", appId);
        const auto resp = ac::http::GetUnchecked(url, constants::kDonorResolveTimeoutSec);

        bool ok = false;
        if (resp.status == 200 && !resp.body.empty()) {
            try {
                const std::uint64_t id = std::stoull(resp.body);
                if (id != 0) {
                    // Positive result: store in cache with TTL
                    g_state.achievements.apiCache.Put(appId, id);
                    AC_LOG_INFO(kModule, "Donor resolved (background) AppID %u -> %llu", appId, id);
                    ok = true;
                } else {
                    // Negative result (id=0): cache as negative
                    g_state.achievements.apiCache.PutNegative(appId);
                    AC_LOG_DEBUG(kModule, "Donor resolved AppID %u -> 0 (cached negative)", appId);
                }
            } catch (...) {
                // Parse error: cache as negative
                g_state.achievements.apiCache.PutNegative(appId);
                AC_LOG_WARN(kModule, "Donor API invalid body AppID %u (cached negative).", appId);
            }
        } else {
            // HTTP error (403/404/timeout): cache as negative
            g_state.achievements.apiCache.PutNegative(appId);
            AC_LOG_DEBUG(kModule, "Donor API failed AppID %u status=%d (cached negative).", appId, resp.status);
        }
        ok ? ++s_donorResolvesDone : ++s_donorResolvesFailed;

        {
            std::lock_guard<std::mutex> lock(s_donorMutex);
            s_donorInflight.erase(appId);
        }
    }
}

void ScheduleDonorResolve(steam::AppId appId) {
    if (appId == 0) return;
    {
        std::lock_guard<std::mutex> lock(s_donorMutex);
        if (s_donorInflight.count(appId)) return;  // single-flight
        s_donorInflight.insert(appId);
        s_donorQueue.push_back(appId);
    }
    s_donorCv.notify_one();

    bool expected = false;
    if (s_donorStarted.compare_exchange_strong(expected, true)) {
        s_donorStop.store(false, std::memory_order_relaxed);
        s_donorWorker = std::thread(DonorWorkerMain);
    }
}

// ---------------------------------------------------------------------------
// Resoluzione Donor ID — NON bloccante (A1) con negative caching (A2).
//
//   cache hit (positive)  -> ritorna il donor API risolto;
//   cache hit (negative)  -> donor_id=0 cached, ritorna SUBITO fallback pool;
//   cache miss            -> accoda una risoluzione in background (single-flight) e
//                            ritorna SUBITO il fallback pool round-robin. Nessuna I/O
//                            sul thread chiamante (wire path di Steam).
// ---------------------------------------------------------------------------
std::uint64_t ResolveDonorId(steam::AppId appId) {
    auto& store = g_state.achievements;

    // Priorità 1: cache hit (positive o negative)
    if (auto cached = store.apiCache.Get(appId)) {
        if (*cached == 0) {
            // Negative cache hit: questo app non ha donor, usa fallback pool
            std::unique_lock<std::shared_mutex> lock(store.poolMutex);
            std::size_t idx = store.nextPoolIndex[appId];
            const std::uint64_t fallbackId = kLumaCoreStatSteamIdPool[idx];
            store.nextPoolIndex[appId] = (idx + 1) % 15;
            AC_LOG_DEBUG(kModule, "Uso fallback LumaCore Pool (negative cache) per AppID %u (index %zu) -> %llu",
                         appId, idx, fallbackId);
            return fallbackId;
        }
        // Positive cache hit: ritorna il donor risolto
        return *cached;
    }

    // Priorità 2: cache miss -> risoluzione asincrona in background (mai bloccante)
    ScheduleDonorResolve(appId);

    // Priorità 3: fallback round-robin sul pool LumaCore (immediato)
    std::unique_lock<std::shared_mutex> lock(store.poolMutex);
    std::size_t idx = store.nextPoolIndex[appId];
    const std::uint64_t fallbackId = kLumaCoreStatSteamIdPool[idx];
    store.nextPoolIndex[appId] = (idx + 1) % 15;

    AC_LOG_DEBUG(kModule, "Uso fallback LumaCore Pool per AppID %u (index %zu) -> %llu",
                 appId, idx, fallbackId);
    return fallbackId;
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
    if (!ac::luadata::HasDepot(appId)) {
        return kNoChange;
    }
    
    // Se la richiesta contiene già uno sha_schema, evitiamo lo spoofing per stabilità
    if (req.has_sha_schema() && !req.sha_schema().empty()) {
        return kNoChange;
    }
    
    std::uint64_t donorId = ResolveDonorId(appId);
    req.set_steamid(donorId);
    
    const std::uint32_t size = static_cast<std::uint32_t>(req.ByteSizeLong());
    if (size > outCap || !req.SerializeToArray(out, static_cast<int>(outCap))) {
        return kNoChange;
    }
    
    AC_LOG_INFO(kModule, "Spoofing GetUserStats (eMsg 151) per AppID %u con DonorID %llu", appId, donorId);
    return static_cast<std::int32_t>(size);
}

std::int32_t HandleSendClientGetUserStats(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CMsgClientGetUserStats req;
    if (!req.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        return kNoChange;
    }
    
    steam::AppId appId = static_cast<steam::AppId>(req.game_id());
    if (!ac::luadata::HasDepot(appId)) {
        return kNoChange;
    }
    
    // Forza la richiesta dello schema azzerando la versione locale (ottiene lo schema aggiornato)
    req.set_schema_local_version(-1);
    
    std::uint64_t donorId = ResolveDonorId(appId);
    req.set_steam_id_for_user(donorId);
    
    const std::uint32_t size = static_cast<std::uint32_t>(req.ByteSizeLong());
    if (size > outCap || !req.SerializeToArray(out, static_cast<int>(outCap))) {
        return kNoChange;
    }
    
    AC_LOG_INFO(kModule, "Spoofing ClientGetUserStats (eMsg 818) per AppID %u con DonorID %llu", appId, donorId);
    return static_cast<std::int32_t>(size);
}

// ---------------------------------------------------------------------------
// Normalizzazione delle Risposte in Entrata (HandleRecv)
// ---------------------------------------------------------------------------

std::int32_t HandleRecvGetUserStatsResponse(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap,
                                            std::uint8_t* outHdr, std::uint32_t outHdrCap, std::int32_t* outNewHdrLen) {
    // 1. Modifica dell'Header Protobuf per assicurarne il successo (eresult = k_EResultOK)
    CMsgProtoBufHeader hdrMsg;
    if (!hdrMsg.ParseFromArray(frame.header, static_cast<int>(frame.headerLen))) {
        return kNoChange;
    }
    
    hdrMsg.set_eresult(1); // k_EResultOK
    const std::uint32_t newHdrSize = static_cast<std::uint32_t>(hdrMsg.ByteSizeLong());
    if (newHdrSize > outHdrCap || !hdrMsg.SerializeToArray(outHdr, static_cast<int>(outHdrCap))) {
        return kNoChange;
    }
    *outNewHdrLen = static_cast<std::int32_t>(newHdrSize);
    
    // 2. Modifica del Body per rimuovere gli sblocchi del donatore
    CPlayer_GetUserStats_Response resp;
    if (!resp.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        return kNoChange;
    }
    
    // Svuotamento dei progressi altrui
    resp.clear_stats();
    
    const std::uint32_t size = static_cast<std::uint32_t>(resp.ByteSizeLong());
    if (size > outCap || !resp.SerializeToArray(out, static_cast<int>(outCap))) {
        return kNoChange;
    }
    
    AC_LOG_INFO(kModule, "Risposta GetUserStats riscritta con successo (eresult=OK, stats ripulite).");
    return static_cast<std::int32_t>(size);
}

std::int32_t HandleRecvClientGetUserStatsResponse(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CMsgClientGetUserStatsResponse resp;
    if (!resp.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        return kNoChange;
    }
    
    steam::AppId appId = static_cast<steam::AppId>(resp.game_id());
    if (!ac::luadata::HasDepot(appId)) {
        return kNoChange;
    }
    
    // Svuotamento totale degli sblocchi estranei
    resp.clear_stats();
    resp.clear_achievement_blocks();
    resp.set_eresult(1); // k_EResultOK
    
    const std::uint32_t size = static_cast<std::uint32_t>(resp.ByteSizeLong());
    if (size > outCap || !resp.SerializeToArray(out, static_cast<int>(outCap))) {
        return kNoChange;
    }
    
    AC_LOG_INFO(kModule, "Risposta ClientGetUserStatsResponse (eMsg 819) riscritta (stats rimosse).");
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
    if (!ac::luadata::HasDepot(appId)) {
        return kNoChange;
    }
    
    // Intercettazione degli achievement committati dal gioco
    for (const auto& ach : req.achievement_blocks()) {
        AC_LOG_INFO(kModule, "NOTIFICA EVENTO: Sbloccato Achievement %u per AppID %u!", ach.achievement_id(), appId);
    }
    
    // RISOLUZIONE BUG DI MISMATCH STEAMID:
    // Se il gioco crede che l'utente sia lo SteamID fittizio di spoofing (es. letto dal registro
    // per sbloccare la licenza), tenterà di salvare le stats per quell'ID. Ma il Connection Manager
    // di Steam rifiuterà la scrittura perché la sessione loggata attiva appartiene al vero utente.
    // Risolviamo riscrivendo settor_steam_id e settee_steam_id con il vero SteamID attivo dell'utente.
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

std::size_t PendingDonorResolves() {
    std::lock_guard<std::mutex> lock(s_donorMutex);
    return s_donorInflight.size();
}

void Shutdown() {
    {
        std::lock_guard<std::mutex> lock(s_donorMutex);
        s_donorStop.store(true, std::memory_order_relaxed);
        s_donorQueue.clear();
        s_donorInflight.clear();
    }
    s_donorCv.notify_all();
    if (s_donorWorker.joinable()) s_donorWorker.join();
}

} // namespace ac::hooks::AchievementModule
