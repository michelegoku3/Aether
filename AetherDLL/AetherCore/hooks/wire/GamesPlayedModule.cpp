#include "pch.h"
#include "hooks/wire/GamesPlayedModule.h"

#include <cstdint>
#include <string>
#include <utility>
#include <vector>

#include "scripting/LuaData.h"
#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "hooks/ipc/PipeWatch.h"
#include "hooks/wire/PersonaInject.h"
#include "utils/GameNameResolver.h"

#include "steam_messages.pb.h"

namespace ac::hooks::GamesPlayed {
namespace {

constexpr const char* kModule = "Wire.GamesPlayed";
constexpr std::int32_t kNoChange = -1;

steam::AppId AppIdFromGameId(std::uint64_t gameId) {
    return static_cast<steam::AppId>(gameId & constants::kGameIdAppIdMask);
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

// Walk Steam binary KV1 (type 0x00 struct, 0x01 string, 0x08 end).
void ExtractStringKVs(const std::uint8_t* data, std::uint32_t size,
                      std::vector<std::pair<std::string, std::string>>& out) {
    std::uint32_t pos = 0;
    int depth = 0;
    auto readCStr = [&](std::string& s) -> bool {
        const std::uint32_t start = pos;
        while (pos < size && data[pos] != 0) ++pos;
        if (pos >= size) return false;
        s.assign(reinterpret_cast<const char*>(data + start), pos - start);
        ++pos;
        return true;
    };
    while (pos < size) {
        const std::uint8_t type = data[pos++];
        if (type == 0x08) {
            if (depth > 0) {
                --depth;
                continue;
            }
            break;
        }
        if (type == 0x00) {
            std::string ignored;
            if (!readCStr(ignored)) return;
            ++depth;
        } else if (type == 0x01) {
            std::string key, value;
            if (!readCStr(key) || !readCStr(value)) return;
            out.emplace_back(std::move(key), std::move(value));
        } else {
            return;
        }
    }
}

}  // namespace

std::int32_t HandleSend(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    // Hot-reload settings in real time if aethercore.toml was updated by AetherDesk
    Settings::ReloadIfModified(g_state.configPath);

    CMsgClientGamesPlayed msg;
    if (!msg.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        return kNoChange;
    }

    LearnSelfSteamId(frame);

    // ---- topmost app (banner follows the tail of the stack) ----------------
    steam::AppId topmost = 0;
    if (msg.games_played_size() > 0) {
        const auto& tail = msg.games_played(msg.games_played_size() - 1);
        if (tail.has_game_id()) topmost = AppIdFromGameId(tail.game_id());
    }

    // ---- tracking for local PersonaInject ----------------------------------
    // Only Lua-managed non-owned apps (HasDepot). Owned titles defer to the
    // server's natural broadcast. Spacewar/480 is never the inject target —
    // OnlineFix uses Dual policy: session 480 + local inject of real app via
    // onlineFixRealAppId when present.
    steam::AppId newTracked = 0;
    if (g_state.settings.presenceInjectLocal) {
        if (topmost != 0 && topmost != constants::kSpacewarAppId && luadata::HasDepot(topmost)) {
            newTracked = topmost;
        } else if (topmost == constants::kSpacewarAppId) {
            // OnlineFix session: prefer real app for local presence inject.
            const steam::AppId real = g_state.onlineFixRealAppId.load();
            if (real != 0 && luadata::IsConfigured(real)) newTracked = real;
        }
    }

    const steam::AppId prev = PersonaInject::PlayingApp();
    if (newTracked != prev) {
        if (newTracked != 0) {
            PersonaInject::SetPlayingApp(newTracked);
            std::lock_guard<std::mutex> lock(g_state.presence.mutex);
            ++g_state.presence.gamesPlayedTrackCount;
            AC_LOG_INFO_ONCE(kModule, "Tracking topmost/display appid %u (stack top=%u).",
                        newTracked, topmost);
        } else if (topmost == 0) {
            // Stack empty → clear local presence and reset session tracking
            // so that re-launching the same game re-emits dedup'd logs once.
            PersonaInject::SetPlayingApp(0);
            pipewatch::ResetSessionTracking();
            AC_LOG_DEBUG(kModule, "GamesPlayed empty; clearing local presence.");
        } else {
            // Topmost is legit-owned (or unmanaged): do NOT clear inject —
            // avoids a brief "Online" flicker while the server push lands (OST).
            AC_LOG_DEBUG(kModule,
                         "Topmost appid %u is owned/unmanaged; deferring presence to server.",
                         topmost);
        }
    }

    // ---- game_extra_info (always-on when enabled) --------------------------
    // Unified path: with and without -onlinefix.
    //   OF entry (game_id 480): extra_info = name(real)
    //   normal entry:           extra_info = name(that appid) if we care
    if (!g_state.settings.presenceAlwaysExtraInfo) return kNoChange;

    bool patched = false;
    const steam::AppId ofReal = g_state.onlineFixRealAppId.load();

    for (int i = 0; i < msg.games_played_size(); ++i) {
        auto* game = msg.mutable_games_played(i);
        if (!game->has_game_id()) continue;
        const steam::AppId app = AppIdFromGameId(game->game_id());

        steam::AppId nameApp = 0;
        if (app == constants::kSpacewarAppId && ofReal != 0) {
            // OnlineFix: never rewrite game_id; only annotate.
            nameApp = ofReal;
        } else if (app != 0 && app != constants::kSpacewarAppId) {
            // No-OF (or non-480 entry): optional polish on the real id.
            // Prefer Lua-managed / configured apps to avoid touching unrelated titles.
            if (luadata::IsConfigured(app) || luadata::HasDepot(app)) nameApp = app;
        }
        if (nameApp == 0) continue;

        std::string name;
        if (!g_state.settings.presenceCustomGameName.empty()) {
            name = g_state.settings.presenceCustomGameName;
        } else {
            name = gamename::ForApp(nameApp);
        }
        if (name.empty()) continue;
        if (game->has_game_extra_info() && game->game_extra_info() == name) continue;

        game->set_game_extra_info(name);
        patched = true;
        AC_LOG_INFO_ONCE(kModule, "game_extra_info appid=%u (wire=%u) -> '%s'.", nameApp, app,
                    name.c_str());
    }

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
    ExtractStringKVs(reinterpret_cast<const std::uint8_t*>(raw.data()),
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
