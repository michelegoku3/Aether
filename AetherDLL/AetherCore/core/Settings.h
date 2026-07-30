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

    // Parses the TOML at configPath.
    static Settings Load(const std::string& configPath);
};

}  // namespace ac
