#pragma once

#include <cstdint>
#include <string>
#include <vector>

#include "core/Logger.h"

// ---------------------------------------------------------------------------
// Configuration loaded from <Steam>\aethercore\aethercore.toml.
//
// Schema:
//   [log]
//   level = "info"                  # trace | debug | info | warn | error | off
//   keep_last_session = true        # save previous session as main.log.last on launch
// ---------------------------------------------------------------------------
namespace ac {

struct Settings {
    // [log]
#ifdef AETHERCORE_RELEASE
    LogLevel logLevel = LogLevel::Warn;
#else
    LogLevel logLevel = LogLevel::Debug;
#endif
    bool logKeepLastSession = true;

    // [lua]
    std::vector<std::string> luaExtraPaths;

    // [lua] http_allowlist
    std::vector<std::string> httpAllowlistExtra;

    // [network]
    std::string patternMirror;

    // [manifest_fetch]
    std::vector<std::string> manifestFetchUrls = {
        "https://manifest.opensteamtool.com/{gid}",
        "https://manifest.steam.run/api/manifest/{gid}",
        "http://gmrc.wudrm.com/manifest/{gid}",
    };
    int manifestFetchTimeoutSec = 12;

    std::vector<std::string> manifestFetchTrustedHosts = {
        "manifest.opensteamtool.com",
        "manifest.steam.run",
        "gmrc.wudrm.com",
    };

    // [presence]
    bool presenceInjectLocal = true;
    bool presenceAlwaysExtraInfo = true;
    bool presenceAetherOnlinePersonaPatch = true;
    std::string presenceCustomGameName;
    // -showonline: rewrite the outgoing presence frames of masked sessions
    // (-showonline / -aetheronline) so the server announces Spacewar/480 with the
    // real appid carried as suffix in game_extra_info.
    bool presenceShowOnlineBroadcast = true;
    // Friend side: rebuild a friend's real app id locally — first from the
    // appid suffix the sender embeds in game_extra_info (relayed as
    // Friend.game_name), then from the title via the configured-library
    // reverse lookup. Local view only; recovers the game icon.
    bool presenceFriendAppIdFromName = true;
    // Sender side, primary appid channel: write game_data_blob (raw bytes,
    // relayed as Friend.game_data_blob) instead of any visible suffix, so the
    // name traveling in game_extra_info is the ONLY thing vanilla friends see.
    bool presenceAppIdBlob = true;
    // Sender side, fallback only (used when appid_blob=false): suffix form.
    // MEASURED (2026-08-24): Steam's friends UI rasterises the U+200B +
    // Variation Selectors channel as tofu rectangles — its font lacks those
    // glyphs. Kept as a compatibility knob for legacy interop; default off.
    bool presenceSuffixInvisible = false;

    // Appids that activate a -showonline session WITHOUT any launch argument
    // (docs/05-showonline-suffix-plan.md §11). AetherDesk maintains this list;
    // via SpawnProcess the session resolves purely from config, so the
    // game-visible launch surface stays byte-identical to a normal launch.
    // That designs out the whole class of games that hard-crash parsing
    // argv / launch options strictly (Selene ~Apoptosis~, Z.A.T.O.).
    std::vector<std::uint32_t> presenceShowOnlineApps;

    // Centralised per-app launch policy (docs/05 §12). The DLL resolves ONE
    // mode per app at SpawnProcess — no launch arguments involved:
    //   aetheronline_apps  -> full Spacewar/480 process mask (superset of presence)
    //   exclude_apps    -> hard opt-out, beats tokens and every other array
    // AetherOnline crack-compat note: a self-masking crack needs nothing here;
    // these arrays are for OUR integration.
    std::vector<std::uint32_t> presenceAetherOnlineApps;
    std::vector<std::uint32_t> presenceExcludeApps;
    // Policy default for apps in NO array (intention over enumeration):
    //   default_mode = "showonline" -> presence broadcast for every launch.
    bool presenceDefaultShowOnline = false;

    // Parses the TOML at configPath.
    static Settings Load(const std::string& configPath);

    // Checks if configPath has been modified on disk and reloads settings in memory.
    static void ReloadIfModified(const std::string& configPath);
};

}  // namespace ac
