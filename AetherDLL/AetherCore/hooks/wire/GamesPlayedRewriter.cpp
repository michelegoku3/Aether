#include "pch.h"
#include "hooks/wire/GamesPlayedRewriter.h"

#include <string>

#include "core/Constants.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "hooks/wire/GamesPlayedFormat.h"
#include "utils/GameNameResolver.h"

#include "steam_messages.pb.h"

namespace ac::hooks::GamesPlayedRewriter {
namespace {

constexpr const char* kModule = "Wire.GamesPlayed";

// Name shown in game_extra_info: the user's custom override when set,
// else the localized title resolved through Steam's own AppInfo cache.
std::string DisplayName(steam::AppId appId, const Settings& settings) {
    if (appId == 0 || appId == constants::kSpacewarAppId) return {};
    if (!settings.presenceCustomGameName.empty()) {
        return settings.presenceCustomGameName;
    }
    const std::string name = gamename::ForApp(appId);
    return GamesPlayedFormat::NameIsUsable(name) ? name : std::string{};
}

// Annotates a MASKED (wire 480) games_played entry. Layers:
//   1) game_data_blob when enabled (raw bytes; CM recycling UNVERIFIED — one
//      field test 16:38 2026-08-24 suggests the CM strips it from Friend relay)
//   2) plan B (always for -showonline): the real appid packed into game_id
//      bits 32-63. Vanilla UIs key on low-24 appid bits (480) and the
//      extra_info text only; mod bits are not rendered. Does not apply to
//      AetherOnline entries (their gid bits may feed OF's own discovery).
void AnnotateMaskedEntry(CMsgClientGamesPlayed::GamePlayed& game, const std::string& name,
                         steam::AppId appId, bool packGameIdHighBits, const Settings& settings) {
    if (packGameIdHighBits) {
        game.set_game_id((game.game_id() & 0x00000000FFFFFFFFull) |
                         (static_cast<std::uint64_t>(appId) << 32));
    }
    if (settings.presenceAppIdBlob) {
        game.set_game_data_blob(GamesPlayedFormat::MakeAppIdBlob(appId));
        game.set_game_extra_info(name);
        return;
    }
    game.set_game_extra_info(GamesPlayedFormat::WithAppIdSuffix(name, appId, settings));
}

}  // namespace

bool Rewrite(CMsgClientGamesPlayed& msg, const RewriteContext& ctx) {
    const Settings& settings = ctx.settings;
    bool patched = false;

    // ---- -showonline wire presence rewrite ----------------------------------
    // The -showonline process keeps its real appid everywhere locally (set by
    // h_SpawnProcess, never masked); only this outbound frame is rewritten so
    // the server announces the session exactly like an -aetheronline mask: appid
    // bits -> 480, and the real appid hidden via AnnotateMaskedEntry
    // (game_data_blob by default — invisible to vanilla friends; suffix
    // fallback otherwise, see docs/05 §9-§10). PersonaInject decodes both.
    // Gated by presenceShowOnlineBroadcast, independent of always_extra_info.
    if (ctx.soSession != 0 && ctx.soSession != constants::kSpacewarAppId &&
        settings.presenceShowOnlineBroadcast) {
        const std::string soName = DisplayName(ctx.soSession, settings);
        for (int i = 0; i < msg.games_played_size(); ++i) {
            auto* game = msg.mutable_games_played(i);
            if (!game->has_game_id()) continue;
            if (GamesPlayedFormat::AppIdFromGameId(game->game_id()) != ctx.soSession) continue;

            // Rewrite ONLY the appid bits; type/owner bits are preserved.
            game->set_game_id((game->game_id() & ~constants::kGameIdAppIdMask) |
                              static_cast<std::uint64_t>(constants::kSpacewarAppId));
            if (!soName.empty()) {
                AnnotateMaskedEntry(*game, soName, ctx.soSession, /*packGameIdHighBits=*/true, settings);
            }
            patched = true;
            const char* channel = settings.presenceAppIdBlob
                                      ? "blob (game_data_blob; extra_info = plain name)"
                                      : settings.presenceSuffixInvisible
                                          ? "invisible suffix"
                                          : "ascii suffix";
            AC_LOG_INFO_ONCE(kModule,
                             "showonline: games_played %u -> 480 (name '%s', channel=%s); "
                             "the process stays registered under the real appid.",
                             ctx.soSession, soName.c_str(), channel);
        }
    }

    // ---- game_extra_info (always-on when enabled) --------------------------
    // Unified path: with and without -aetheronline.
    //   OF/masked entry (480):  AnnotateMaskedEntry (blob default + plain name)
    //   normal entry:           extra_info = name(that appid) if we care
    // Entries just rewritten by the -showonline block above (480 without an OF
    // session) are untouched here: their extra_info is already set.
    if (settings.presenceAlwaysExtraInfo) {
        for (int i = 0; i < msg.games_played_size(); ++i) {
            auto* game = msg.mutable_games_played(i);
            if (!game->has_game_id()) continue;
            const steam::AppId app = GamesPlayedFormat::AppIdFromGameId(game->game_id());

            if (ctx.foreignSpoof) {
                if (app != constants::kSpacewarAppId) continue;
                const steam::AppId nameApp = ctx.spoofReal;
                const std::string name = DisplayName(nameApp, settings);
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
            const std::string name = DisplayName(app, settings);
            if (name.empty()) continue;
            if (game->has_game_extra_info() && game->game_extra_info() == name) continue;
            game->set_game_extra_info(name);
            patched = true;
            AC_LOG_INFO_ONCE(kModule, "game_extra_info appid=%u (wire=%u) -> '%s'.",
                             app, app, name.c_str());
        }
    }

    return patched;
}

}  // namespace ac::hooks::GamesPlayedRewriter
