#include "pch.h"
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

std::string ImageStem(const std::string& imageName) {
    const auto slash = imageName.find_last_of("\\/");
    std::string file = (slash == std::string::npos) ? imageName : imageName.substr(slash + 1);
    const auto dot = file.find_last_of('.');
    if (dot != std::string::npos) file.resize(dot);
    // Unreal: Bodycam-Win64-Shipping -> Bodycam
    static constexpr const char* kShipping[] = {
        "-Win64-Shipping", "-Win32-Shipping", "-Win64-Test", "-Win32-Test",
    };
    for (const char* suf : kShipping) {
        const std::size_t n = std::strlen(suf);
        if (file.size() > n) {
            const std::string tail = file.substr(file.size() - n);
            std::string fold = tail;
            for (char& c : fold) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
            std::string want = suf;
            for (char& c : want) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
            if (fold == want) {
                file.resize(file.size() - n);
                break;
            }
        }
    }
    // ReadyOrNotSteam-Win64-Shipping -> ReadyOrNot
    if (file.size() > 5) {
        std::string tail = file.substr(file.size() - 5);
        for (char& c : tail) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
        if (tail == "steam") file.resize(file.size() - 5);
    }
    return file;
}

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
    const steam::AppId ofReal = g_state.onlineFixRealAppId.load();
    if (ofReal != 0 && ofReal != constants::kSpacewarAppId) return ofReal;

    const steam::AppId spawned = g_state.lastSpawnedAppId.load();
    if (spawned != 0 && spawned != constants::kSpacewarAppId) return spawned;

    std::lock_guard<std::mutex> lock(g_state.pipeWatch.mutex);
    for (const auto& entry : g_state.pipeWatch.snapshots) {
        const auto& snap = entry.second;
        if (!SnapshotLooksSpoofed(snap)) continue;
        const std::string stem = ImageStem(snap.imageName.empty() ? snap.imagePath : snap.imageName);
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

// Name shown in game_extra_info: the user's custom override when set,
// else the localized title resolved through Steam's own AppInfo cache.
bool NameIsUsable(const std::string& name) {
    if (name.empty()) return false;
    std::string fold;
    fold.reserve(name.size());
    for (unsigned char c : name) {
        if (std::isspace(c)) continue;
        fold.push_back(static_cast<char>(std::tolower(c)));
    }
    return fold != "spacewar";
}

std::string DisplayName(steam::AppId appId) {
    if (appId == 0 || appId == constants::kSpacewarAppId) return {};
    if (!g_state.settings.presenceCustomGameName.empty()) {
        return g_state.settings.presenceCustomGameName;
    }
    const std::string name = gamename::ForApp(appId);
    return NameIsUsable(name) ? name : std::string{};
}

// game_extra_info is the one field the CM reliably recycles into
// Friend.game_name for masked (480) sessions (measured, see
// docs/04-showonline-plan.md §1). We hide the exact appid at its tail so the
// friend's AetherDLL can recover it deterministically. Default form is the
// invisible channel (U+200B + 6 VS nibbles, constants::kExtraInfoInvisible*):
// vanilla friends see ONLY the clean human name; the ASCII form
// "<name> | <appid>" remains for legacy receivers (suffix_invisible=false).
std::string WithAppIdSuffix(const std::string& name, steam::AppId appId) {
    if (!g_state.settings.presenceSuffixInvisible) {
        return name + constants::kExtraInfoAppIdSep + std::to_string(appId);
    }
    std::string out = name + constants::kExtraInfoInvisibleMark;
    constexpr std::size_t digits = constants::kExtraInfoInvisibleDigits;
    for (std::size_t i = 0; i < digits; ++i) {
        const int shift = static_cast<int>((digits - 1 - i) * 4);
        const std::uint8_t nib = static_cast<std::uint8_t>((appId >> shift) & 0xF);
        out += "\xEE\xB8";
        out.push_back(static_cast<char>(0x80 | nib));
    }
    return out;
}

// Preferred appid carrier (docs/05 §10): pack the appid into
// GamePlayed.game_data_blob — raw bytes the CM recycles into
// Friend.game_data_blob for masked sessions. It is never rendered by any
// client UI: vanilla friends see ONLY the plain name from game_extra_info.
std::string MakeAppIdBlob(steam::AppId appId) {
    std::string b(constants::kAppIdBlobMagic, 4);
    b.push_back(static_cast<char>(constants::kAppIdBlobVersion));
    b.push_back(static_cast<char>(appId & 0xFF));
    b.push_back(static_cast<char>((appId >> 8) & 0xFF));
    b.push_back(static_cast<char>((appId >> 16) & 0xFF));
    b.push_back(static_cast<char>((appId >> 24) & 0xFF));
    return b;
}

// Annotates a MASKED (wire 480) games_played entry. Layers:
//   1) game_data_blob when enabled (raw bytes; CM recycling UNVERIFIED — one
//      field test 16:38 2026-08-24 suggests the CM strips it from Friend relay)
//   2) plan B (always for -showonline): the real appid packed into game_id
//      bits 32-63. Vanilla UIs key on low-24 appid bits (480) and the
//      extra_info text only; mod bits are not rendered. Does not apply to
//      OnlineFix entries (their gid bits may feed OF's own discovery).
void AnnotateMaskedEntry(CMsgClientGamesPlayed::GamePlayed& game, const std::string& name,
                         steam::AppId appId, bool packGameIdHighBits) {
    if (packGameIdHighBits) {
        game.set_game_id((game.game_id() & 0x00000000FFFFFFFFull) |
                         (static_cast<std::uint64_t>(appId) << 32));
    }
    if (g_state.settings.presenceAppIdBlob) {
        game.set_game_data_blob(MakeAppIdBlob(appId));
        game.set_game_extra_info(name);
        return;
    }
    game.set_game_extra_info(WithAppIdSuffix(name, appId));
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

    steam::AppId topmost = 0;
    bool stackHas480 = false;
    if (msg.games_played_size() > 0) {
        const auto& tail = msg.games_played(msg.games_played_size() - 1);
        if (tail.has_game_id()) topmost = AppIdFromGameId(tail.game_id());
        for (int i = 0; i < msg.games_played_size(); ++i) {
            if (AppIdFromGameId(msg.games_played(i).game_id()) == constants::kSpacewarAppId) {
                stackHas480 = true;
                break;
            }
        }
    }

    const steam::AppId spoofReal = RealAppForSpoofedSession();
    // Foreign DLL (UCO2/OFME) owns Spacewar: known at spawn, or 480 already
    // on the stack. Aether then only writes extra_info on that 480.
    const bool foreignSpoof = g_state.spacewarSpoofExpected.load()
        || (stackHas480 && spoofReal != 0);

    if (foreignSpoof) {
        g_state.showOnlineAppId.store(0);
        if (PersonaInject::PlayingApp() != 0) PersonaInject::SetPlayingApp(0);
    } else if (g_state.settings.presenceInjectLocal
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

    // [DIAG] Cosa stiamo realmente annunciando al CM, loggato solo su
    // variazione (GamesPlayed e' periodico).
    {
        static std::atomic<std::uint64_t> s_lastTxSig{~0ull};
        std::uint64_t sig = static_cast<std::uint64_t>(msg.games_played_size());
        for (int i = 0; i < msg.games_played_size(); ++i) {
            sig = sig * 1000003ull + msg.games_played(i).game_id();
        }
        if (s_lastTxSig.exchange(sig) != sig) {
            for (int i = 0; i < msg.games_played_size(); ++i) {
                const auto& g = msg.games_played(i);
                AC_LOG_INFO(kModule,
                            "[DIAG] TX[%d] game_id=%llu (app=%u) extra='%s' "
                            "owner_id=%u process_id=%u game_flags=%u",
                            i, static_cast<unsigned long long>(g.game_id()),
                            AppIdFromGameId(g.game_id()),
                            g.game_extra_info().c_str(), g.owner_id(),
                            g.process_id(), g.game_flags());
            }
            if (msg.games_played_size() == 0) {
                AC_LOG_INFO(kModule, "[DIAG] TX: games_played vuoto (uscita dal gioco).");
            }
        }
    }

    // ---- -showonline wire presence rewrite ----------------------------------
    // The -showonline process keeps its real appid everywhere locally (set by
    // h_SpawnProcess, never masked); only this outbound frame is rewritten so
    // the server announces the session exactly like an -onlinefix mask: appid
    // bits -> 480, and the real appid hidden via AnnotateMaskedEntry
    // (game_data_blob by default — invisible to vanilla friends; suffix
    // fallback otherwise, see docs/05 §9-§10). PersonaInject decodes both.
    // Gated by presenceShowOnlineBroadcast, independent of always_extra_info.
    bool patched = false;

    const steam::AppId soSession = foreignSpoof ? 0 : g_state.showOnlineAppId.load();
    if (soSession != 0 && soSession != constants::kSpacewarAppId &&
        g_state.settings.presenceShowOnlineBroadcast) {
        const std::string soName = DisplayName(soSession);
        for (int i = 0; i < msg.games_played_size(); ++i) {
            auto* game = msg.mutable_games_played(i);
            if (!game->has_game_id()) continue;
            if (AppIdFromGameId(game->game_id()) != soSession) continue;

            // Rewrite ONLY the appid bits; type/owner bits are preserved.
            game->set_game_id((game->game_id() & ~constants::kGameIdAppIdMask) |
                              static_cast<std::uint64_t>(constants::kSpacewarAppId));
            if (!soName.empty()) {
                AnnotateMaskedEntry(*game, soName, soSession, /*packGameIdHighBits=*/true);
            }
            patched = true;
            const char* channel = g_state.settings.presenceAppIdBlob
                                      ? "blob (game_data_blob; extra_info = plain name)"
                                      : g_state.settings.presenceSuffixInvisible
                                          ? "invisible suffix"
                                          : "ascii suffix";
            AC_LOG_INFO_ONCE(kModule,
                             "showonline: games_played %u -> 480 (name '%s', channel=%s); "
                             "the process stays registered under the real appid.",
                             soSession, soName.c_str(), channel);
        }
    }

    // ---- game_extra_info (always-on when enabled) --------------------------
    // Unified path: with and without -onlinefix.
    //   OF/masked entry (480):  AnnotateMaskedEntry (blob default + plain name)
    //   normal entry:           extra_info = name(that appid) if we care
    // Entries just rewritten by the -showonline block above (480 without an OF
    // session) are untouched here: their extra_info is already set.
    if (g_state.settings.presenceAlwaysExtraInfo) {
        for (int i = 0; i < msg.games_played_size(); ++i) {
            auto* game = msg.mutable_games_played(i);
            if (!game->has_game_id()) continue;
            const steam::AppId app = AppIdFromGameId(game->game_id());

            if (foreignSpoof) {
                if (app != constants::kSpacewarAppId) continue;
                const steam::AppId nameApp = spoofReal;
                const std::string name = DisplayName(nameApp);
                if (name.empty()) continue;
                if (game->has_game_data_blob()) {
                    game->clear_game_data_blob();
                    patched = true;
                }
                if ((game->game_id() >> 32) != 0) {
                    game->set_game_id(game->game_id() & 0x00000000FFFFFFFFull);
                    patched = true;
                }
                if (!game->has_game_extra_info() || game->game_extra_info() != name) {
                    game->set_game_extra_info(name);
                    patched = true;
                }
                AC_LOG_INFO_ONCE(kModule, "game_extra_info spoof 480 -> '%s' (real=%u).",
                                 name.c_str(), nameApp);
                continue;
            }

            if (app == 0 || app == constants::kSpacewarAppId) continue;
            if (!luadata::IsConfigured(app) && !luadata::HasDepot(app)) continue;
            const std::string name = DisplayName(app);
            if (name.empty()) continue;
            if (game->has_game_extra_info() && game->game_extra_info() == name) continue;
            game->set_game_extra_info(name);
            patched = true;
            AC_LOG_INFO_ONCE(kModule, "game_extra_info appid=%u (wire=%u) -> '%s'.",
                             app, app, name.c_str());
        }
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
