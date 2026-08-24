#include "pch.h"
#include "core/Settings.h"

#include <filesystem>
#include <toml++/toml.hpp>
#include "core/AetherCoreState.h"

namespace ac {

void Settings::ReloadIfModified(const std::string& configPath) {
    if (configPath.empty()) return;
    std::error_code ec;
    auto lastWrite = std::filesystem::last_write_time(configPath, ec);
    if (ec) return;

    static std::filesystem::file_time_type s_lastLoadedTime{};
    if (s_lastLoadedTime == std::filesystem::file_time_type{}) {
        s_lastLoadedTime = lastWrite;
        return;
    }
    if (lastWrite != s_lastLoadedTime) {
        s_lastLoadedTime = lastWrite;
        g_state.settings = Settings::Load(configPath);
        AC_LOG_INFO("Settings", "Hot-reloaded settings from %s (custom_game_name='%s').",
                    configPath.c_str(), g_state.settings.presenceCustomGameName.c_str());
    }
}

Settings Settings::Load(const std::string& configPath) {
    Settings s;  // Start from defaults; only override what the file provides.

    toml::table tbl;
    try {
        tbl = toml::parse_file(configPath);
    } catch (const toml::parse_error&) {
        AC_LOG_INFO("Settings", "No usable config at %s; using defaults.", configPath.c_str());
        return s;
    }

    // [log]
    if (auto level = tbl["log"]["level"].value<std::string>()) {
        s.logLevel = log::ParseLevel(*level, s.logLevel);
    }
    if (auto keep = tbl["log"]["keep_last_session"].value<bool>()) {
        s.logKeepLastSession = *keep;
    }
    // [lua]
    if (auto* paths = tbl["lua"]["extra_paths"].as_array()) {
        for (const auto& node : *paths) {
            if (auto p = node.value<std::string>(); p && !p->empty()) {
                s.luaExtraPaths.push_back(*p);
            }
        }
    }
    if (auto* hosts = tbl["lua"]["http_allowlist"].as_array()) {
        for (const auto& node : *hosts) {
            if (auto h = node.value<std::string>(); h && !h->empty()) {
                s.httpAllowlistExtra.push_back(*h);
            }
        }
    }

    // [network]
    if (auto mirror = tbl["network"]["pattern_mirror"].value<std::string>()) {
        s.patternMirror = *mirror;
    }

    // [manifest_fetch]
    if (auto* mfetch = tbl["manifest_fetch"].as_table()) {
        if (auto* urls = (*mfetch)["urls"].as_array()) {
            s.manifestFetchUrls.clear();
            for (const auto& node : *urls) {
                if (auto u = node.value<std::string>(); u && !u->empty()) {
                    s.manifestFetchUrls.push_back(*u);
                }
            }
        } else if (auto url = (*mfetch)["url"].value<std::string>()) {
            s.manifestFetchUrls = { *url };
        }
        if (auto secs = (*mfetch)["timeout_sec"].value<int>()) {
            if (*secs > 0) s.manifestFetchTimeoutSec = *secs;
        }
        if (auto* hosts = (*mfetch)["trusted_hosts"].as_array()) {
            s.manifestFetchTrustedHosts.clear();
            for (const auto& node : *hosts) {
                if (auto h = node.value<std::string>(); h && !h->empty()) {
                    s.manifestFetchTrustedHosts.push_back(*h);
                }
            }
        }
    }

    // [presence]
    if (auto* presence = tbl["presence"].as_table()) {
        if (auto v = (*presence)["inject_local"].value<bool>()) {
            s.presenceInjectLocal = *v;
        }
        if (auto v = (*presence)["always_extra_info"].value<bool>()) {
            s.presenceAlwaysExtraInfo = *v;
        }
        if (auto v = (*presence)["onlinefix_persona_patch"].value<bool>()) {
            s.presenceOnlineFixPersonaPatch = *v;
        }
        if (auto v = (*presence)["custom_game_name"].value<std::string>()) {
            s.presenceCustomGameName = *v;
        }
        if (auto v = (*presence)["showonline_broadcast"].value<bool>()) {
            s.presenceShowOnlineBroadcast = *v;
        }
        if (auto v = (*presence)["friend_appid_from_name"].value<bool>()) {
            s.presenceFriendAppIdFromName = *v;
        }
    }

    AC_LOG_INFO("Settings",
                "Loaded %s (keep_last_session=%d, lua extra paths: %zu, "
                "mirror: %s, manifest urls: %zu).",
                configPath.c_str(), s.logKeepLastSession ? 1 : 0,
                s.luaExtraPaths.size(),
                s.patternMirror.empty() ? "default" : "custom",
                s.manifestFetchUrls.size());
    return s;
}

}  // namespace ac
