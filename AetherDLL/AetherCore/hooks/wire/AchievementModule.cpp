#include "pch.h"
#include "hooks/wire/AchievementModule.h"

#include <shared_mutex>
#include <mutex>
#include <chrono>
#include <string>

#include "core/AetherCoreState.h"
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
// Risoluzione Sicura e Dinamica dello SteamID Donatore (OST API + Luma Pool)
// ---------------------------------------------------------------------------
std::uint64_t ResolveDonorId(steam::AppId appId) {
    auto& store = g_state.achievements;
    
    // Scrittura/Lettura condivisa protetta
    std::shared_lock<std::shared_mutex> readLock(store.mutex);
    
    // Priorità 1: Controllo della Cache dei risultati API di OpenSteamTool
    auto cacheIt = store.apiCache.find(appId);
    if (cacheIt != store.apiCache.end() && cacheIt->second != 0) {
        return cacheIt->second;
    }
    
    // Rilascio del lock di lettura prima di effettuare la chiamata HTTP (I/O)
    // per non bloccare altri thread di rete
    readLock.unlock();
    
    // Se l'API di OpenSteamTool è abilitata in aethercore.toml
    if (g_state.settings.statsEnableApi) {
        std::string url = "https://stats.opensteamtool.com/" + std::to_string(appId);
        AC_LOG_INFO(kModule, "Interrogazione API OpenSteamTool per AppID %u...", appId);
        
        // Chiamata HTTP interna non bloccata da liste di controllo
        auto resp = ac::http::GetUnchecked(url, 5); // Timeout 5 secondi
        if (resp.status == 200 && !resp.body.empty()) {
            try {
                std::uint64_t resolvedId = std::stoull(resp.body);
                if (resolvedId != 0) {
                    std::unique_lock<std::shared_mutex> writeLock(store.mutex);
                    store.apiCache[appId] = resolvedId;
                    AC_LOG_INFO(kModule, "API risolto con successo per AppID %u -> %llu", appId, resolvedId);
                    return resolvedId;
                }
            } catch (...) {
                AC_LOG_WARN(kModule, "Risposta API non valida per AppID %u: %s", appId, resp.body.c_str());
            }
        } else {
            AC_LOG_WARN(kModule, "Fallimento chiamata API OpenSteamTool (status=%d)", resp.status);
        }
    }
    
    // Priorità 3: Fallback Round-Robin sul pool di 15 SteamID di LumaCore
    std::unique_lock<std::shared_mutex> writeLock(store.mutex);
    std::size_t idx = store.nextPoolIndex[appId];
    std::uint64_t fallbackId = kLumaCoreStatSteamIdPool[idx];
    
    // Avanzamento indice circolare per l'AppID corrente
    store.nextPoolIndex[appId] = (idx + 1) % 15;
    
    AC_LOG_DEBUG(kModule, "Uso fallback LumaCore Pool per AppID %u (index %zu) -> %llu", appId, idx, fallbackId);
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
    if (!ac::luadata::IsStatsManagedApp(appId)) {
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
    if (!ac::luadata::IsStatsManagedApp(appId)) {
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
    if (!ac::luadata::IsStatsManagedApp(appId)) {
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
    if (!ac::luadata::IsStatsManagedApp(appId)) {
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

} // namespace ac::hooks::AchievementModule
