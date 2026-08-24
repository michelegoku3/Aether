#pragma once

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
    bool presenceOnlineFixPersonaPatch = true;
    std::string presenceCustomGameName;
    // -showonline: rewrite the outgoing presence frames of masked sessions
    // (-showonline / -onlinefix) so the server announces Spacewar/480 with the
    // real appid carried as suffix in game_extra_info.
    bool presenceShowOnlineBroadcast = true;
    // Friend side: rebuild a friend's real app id locally — first from the
    // appid suffix the sender embeds in game_extra_info (relayed as
    // Friend.game_name), then from the title via the configured-library
    // reverse lookup. Local view only; recovers the game icon.
    bool presenceFriendAppIdFromName = true;

    // Parses the TOML at configPath.
    static Settings Load(const std::string& configPath);

    // Checks if configPath has been modified on disk and reloads settings in memory.
    static void ReloadIfModified(const std::string& configPath);
};

}  // namespace ac
