#include "pch.h"
#include "utils/LogBurstBudget.h"
#include "hooks/wire/GamesPlayedModule.h"

#include <atomic>
#include <cctype>
#include <cstdint>
#include <cstring>
#include <string>
#include <utility>
#include <vector>

#include "scripting/LuaData.h"
#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "hooks/aetheronline/PresenceSession.h"
#include "hooks/ipc/PipeWatch.h"
#include "hooks/wire/AchievementBackup.h"
#include "hooks/wire/GamesPlayedFormat.h"
#include "hooks/wire/GamesPlayedRewriter.h"
#include "hooks/wire/PersonaInject.h"
#include "utils/GameNameResolver.h"

#include "steam_messages.pb.h"

namespace ac::hooks::GamesPlayed {
namespace {

namespace fmt = GamesPlayedFormat;

constexpr const char* kModule = "Wire.GamesPlayed";
constexpr std::int32_t kNoChange = -1;

bool SnapshotLooksSpoofed(const pipewatch::ProcessSnapshot& snap) {
    if (!snap.likelyGame || snap.steamProcess) return false;
    return snap.appId == constants::kSpacewarAppId
        || snap.envSteamOverlayGameId == constants::kSpacewarAppId
        || snap.envSteamAppId == constants::kSpacewarAppId
        || snap.envSteamGameId == constants::kSpacewarAppId;
}

// Real app behind a Spacewar (480) session: Online Aether first, then UCO2/OFME
// recovered from the live pipe image (GetAppID reports 480; the exe name does not).
steam::AppId RealAppForSpoofedSession() {
    const steam::AppId ofReal = presence::RealAppId();
    if (ofReal != 0 && ofReal != constants::kSpacewarAppId) return ofReal;

    const steam::AppId spawned = g_state.lastSpawnedAppId.load();
    if (spawned != 0 && spawned != constants::kSpacewarAppId) return spawned;

    std::lock_guard<std::mutex> lock(g_state.pipeWatch.mutex);
    for (const auto& entry : g_state.pipeWatch.snapshots) {
        const auto& snap = entry.second;
        if (!SnapshotLooksSpoofed(snap)) continue;
        const std::string stem = fmt::ImageStem(snap.imageName.empty() ? snap.imagePath : snap.imageName);
        if (stem.empty()) continue;
        const steam::AppId byName = gamename::ResolveAppIdByName(stem);
        if (byName != 0 && byName != constants::kSpacewarAppId) return byName;
    }
    return 0;
}

void LearnSelfSteamId(const WireFrame& frame) {
    if (!frame.header || frame.headerLen == 0) return;
    CMsgProtoBufHeader hdr;
    if (!hdr.ParseFromArray(frame.header, static_cast<int>(frame.headerLen))) return;
    if (!hdr.has_steamid() || hdr.steamid() == 0) return;

    std::lock_guard<std::mutex> lock(g_state.presence.mutex);
    if (g_state.presence.selfSteamId != hdr.steamid()) {
        g_state.presence.selfSteamId = hdr.steamid();
        AC_LOG_DEBUG(kModule, "Captured local SteamID 0x%llX.",
                     static_cast<unsigned long long>(hdr.steamid()));
    }
}

}  // namespace

std::int32_t HandleSend(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    const auto settings = Settings::Snapshot();
    // Retain one immutable configuration for this frame; DirWatch owns reload.

    CMsgClientGamesPlayed msg;
    if (!msg.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        return kNoChange;
    }

    LearnSelfSteamId(frame);

    steam::AppId topmost = 0;
    bool stackHas480 = false;
    if (msg.games_played_size() > 0) {
        const auto& tail = msg.games_played(msg.games_played_size() - 1);
        if (tail.has_game_id()) topmost = fmt::AppIdFromGameId(tail.game_id());
        for (int i = 0; i < msg.games_played_size(); ++i) {
            if (fmt::AppIdFromGameId(msg.games_played(i).game_id()) == constants::kSpacewarAppId) {
                stackHas480 = true;
                break;
            }
        }
    }

    const steam::AppId spoofReal = RealAppForSpoofedSession();
    // Foreign DLL (UCO2/OFME) owns Spacewar: known at spawn, or 480 already
    // on the stack. Aether then only writes extra_info on that 480.
    const bool foreignSpoof = presence::SpacewarSpoofExpected()
        || (stackHas480 && spoofReal != 0);

    const steam::AppId prevPlaying = PersonaInject::PlayingApp();
    if (foreignSpoof) {
        presence::ClearShowOnline();
        if (PersonaInject::PlayingApp() != 0) PersonaInject::SetPlayingApp(0);
    } else if (settings->presenceInjectLocal
               && topmost != 0 && topmost != constants::kSpacewarAppId
               && luadata::HasDepot(topmost)) {
        if (PersonaInject::PlayingApp() != topmost) {
            PersonaInject::SetPlayingApp(topmost);
            std::lock_guard<std::mutex> lock(g_state.presence.mutex);
            ++g_state.presence.gamesPlayedTrackCount;
        }
    } else if (topmost == 0 && PersonaInject::PlayingApp() != 0) {
        PersonaInject::SetPlayingApp(0);
        pipewatch::ResetSessionTracking();
    }

    // Reset della sessione di presenza quando Steam non riporta più nessun
    // gioco in esecuzione: elimina lo stato "sporco" che prima sopravviveva
    // fino allo SpawnProcess successivo (P6).
    if (topmost == 0) {
        presence::EndSession();
    }

    // Persistenza guidata dagli eventi: qualunque transizione che TERMINA una
    // sessione giocata (uscita, cambio gioco, mask 480) schedula la copia
    // finale degli stats dell'app appena chiuso (ritardata: Steam scrive la
    // cache dopo il frame vuoto).
    if (prevPlaying != 0 && prevPlaying != PersonaInject::PlayingApp()) {
        std::uint64_t selfId = 0;
        {
            std::lock_guard<std::mutex> lock(g_state.presence.mutex);
            selfId = g_state.presence.selfSteamId;
        }
        AchievementBackup::SessionEnded(prevPlaying, selfId);
    }

    // [DIAG] Cosa stiamo realmente annunciando al CM, loggato solo su
    // variazione (GamesPlayed e' periodico).
    {
        static std::atomic<std::uint64_t> s_lastTxSig{~0ull};
        std::uint64_t sig = static_cast<std::uint64_t>(msg.games_played_size());
        for (int i = 0; i < msg.games_played_size(); ++i) {
            sig = sig * 1000003ull + msg.games_played(i).game_id();
        }
        if (s_lastTxSig.exchange(sig) != sig) {
            AC_LOG_INFO(kModule, "GamesPlayed state changed: entries=%d topmost_app=%u.",
                        msg.games_played_size(), topmost);
            static logutil::LogBurstBudget detailBudget;
            for (int i = 0; i < msg.games_played_size(); ++i) {
                if (!log::Enabled(LogLevel::Debug)) break;
                const auto decision = detailBudget.Admit();
                if (decision.suppressed) {
                    AC_LOG_DEBUG(kModule, "TX detail budget: skipped %llu attempts in previous window (20/10s).",
                                 static_cast<unsigned long long>(decision.suppressed));
                }
                if (!decision.emit) continue;
                const auto& g = msg.games_played(i);
                AC_LOG_DEBUG(kModule,
                            "[DIAG] TX[%d] game_id=%llu (app=%u) extra='%s' "
                            "owner_id=%u process_id=%u game_flags=%u",
                            i, static_cast<unsigned long long>(g.game_id()),
                            fmt::AppIdFromGameId(g.game_id()),
                            g.game_extra_info().c_str(), g.owner_id(),
                            g.process_id(), g.game_flags());
            }
            if (msg.games_played_size() == 0) {
                AC_LOG_INFO(kModule, "[DIAG] TX: games_played vuoto (uscita dal gioco).");
            }
        }
    }

    // ---- riscrittura presenza (P9: un unico rewriter puro) -----------------
    // I due passaggi storici (-showonline mask + game_extra_info) vivono ora in
    // GamesPlayedRewriter::Rewrite, deterministici in (msg, ctx): qui resta solo
    // la costruzione del contesto dallo snapshot di presenza + settings.
    GamesPlayedRewriter::RewriteContext rwCtx{
        .settings = *settings,
        .soSession = foreignSpoof ? 0 : presence::ShowOnlineAppId(),
        .foreignSpoof = foreignSpoof,
        .spoofReal = spoofReal,
    };
    const bool patched = GamesPlayedRewriter::Rewrite(msg, rwCtx);

    if (!patched) return kNoChange;

    {
        std::lock_guard<std::mutex> lock(g_state.presence.mutex);
        ++g_state.presence.extraInfoPatchCount;
    }

    const std::uint32_t size = static_cast<std::uint32_t>(msg.ByteSizeLong());
    if (size > outCap || !msg.SerializeToArray(out, static_cast<int>(outCap))) {
        AC_LOG_WARN(kModule, "GamesPlayed rewrite too large (%u bytes).", size);
        return kNoChange;
    }
    return static_cast<std::int32_t>(size);
}

std::int32_t HandleRichPresenceUpload(const WireFrame& frame) {
    const steam::AppId playing = PersonaInject::PlayingApp();
    if (playing == 0) return kNoChange;

    CMsgClientRichPresenceUpload up;
    if (!up.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) return kNoChange;
    if (!up.has_rich_presence_kv()) return kNoChange;

    const std::string& raw = up.rich_presence_kv();
    std::vector<std::pair<std::string, std::string>> kvs;
    fmt::ExtractStringKVs(reinterpret_cast<const std::uint8_t*>(raw.data()),
                        static_cast<std::uint32_t>(raw.size()), kvs);

    {
        std::lock_guard<std::mutex> lock(g_state.presence.mutex);
        g_state.presence.rpKvs[playing] = std::move(kvs);
        AC_LOG_DEBUG_ONCE(kModule, "RP upload appid=%u pairs=%zu.", playing,
                     g_state.presence.rpKvs[playing].size());
    }
    PersonaInject::SetPlayingApp(playing, /*forceRestage=*/true);
    return kNoChange;  // never rewrite the outbound upload
}

}  // namespace ac::hooks::GamesPlayed
