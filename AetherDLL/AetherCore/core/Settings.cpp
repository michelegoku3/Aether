#include "pch.h"
#include "core/Settings.h"

#include <atomic>
#include <chrono>
#include <filesystem>
#include <toml++/toml.hpp>
#include "core/AetherCoreState.h"

namespace ac {

namespace {
// Last attempted file revision. Initialized before DirWatch starts; only the
// watcher mutates it at runtime (including rejected malformed revisions).
std::atomic<long long> s_lastConfigWriteTicks{0};

long long FileWriteTicks(const std::string& configPath) {
    std::error_code ec;
    const auto t = std::filesystem::last_write_time(configPath, ec);
    if (ec) return 0;
    return t.time_since_epoch().count();
}
}  // namespace

std::shared_ptr<const Settings> Settings::Snapshot() {
    return g_state.settings.load();
}

bool Settings::Initialize(const std::string& configPath) {
    const auto ticks = FileWriteTicks(configPath);
    bool valid = false;
    auto loaded = std::make_shared<const Settings>(Load(configPath, &valid));
    g_state.settings.store(std::move(loaded));
    s_lastConfigWriteTicks.store(ticks);
    return valid;
}

void Settings::ReloadIfModified(const std::string& configPath) {
    if (configPath.empty()) return;
    const auto ticks = FileWriteTicks(configPath);
    if (ticks == 0 || ticks == s_lastConfigWriteTicks.load()) return;
    // Remember attempted revision too: malformed edits produce one warning,
    // not one warning every second. A subsequent edit is retried normally.
    s_lastConfigWriteTicks.store(ticks);
    bool valid = false;
    auto candidate = std::make_shared<const Settings>(Load(configPath, &valid));
    if (!valid) {
        AC_LOG_WARN("Settings", "Reload rejected; keeping last good configuration (%s).",
                    configPath.c_str());
        return;
    }
    const auto previous = Snapshot();
    g_state.settings.store(candidate);
    log::SetLevel(candidate->logLevel);
    diag::Record("settings_reloaded", configPath);
    AC_LOG_INFO("Settings", "Published immutable configuration snapshot (%s, log_level=%d).",
                configPath.c_str(), static_cast<int>(candidate->logLevel));
    if (candidate->luaExtraPaths != previous->luaExtraPaths) {
        AC_LOG_WARN("Settings", "lua.extra_paths changed: restart Steam to rebuild Lua directory watches.");
    }
    if (candidate->diversionMode != previous->diversionMode) {
        AC_LOG_WARN("Settings", "injection.diversion_mode changed: restart Steam to move the hook target.");
    }
}

Settings Settings::Load(const std::string& configPath, bool* valid) {
    Settings s;
    if (valid) *valid = false;

    toml::table tbl;
    try {
        tbl = toml::parse_file(configPath);
    } catch (const toml::parse_error& e) {
        // Startup falls back to defaults; runtime reload rejects this candidate.
        // The init caller also reports failure AFTER logger initialization.
        AC_LOG_WARN("Settings",
                    "Cannot parse config %s (%s); defaults only apply during startup.",
                    configPath.c_str(), e.what());
        return s;
    }

    // [injection] Parsed at startup, before the module target is selected.
    if (auto mode = tbl["injection"]["diversion_mode"].value<std::string>()) {
        if (*mode == "auto") s.diversionMode = DiversionMode::Auto;
        else if (*mode == "copy") s.diversionMode = DiversionMode::Copy;
        else if (*mode == "live") s.diversionMode = DiversionMode::Live;
        else {
            AC_LOG_WARN("Settings", "Invalid injection.diversion_mode '%s' (auto/copy/live).",
                        mode->c_str());
            return s;  // mark invalid, do not publish a partially parsed hot reload
        }
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
    if (auto useOst = tbl["network"]["use_ost_source"].value<bool>()) {
        s.patternUseOstSource = *useOst;
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
        if (auto ms = (*mfetch)["bridge_wait_ms"].value<int>()) {
            if (*ms >= 0 && *ms <= 10000) s.manifestBridgeWaitMs = *ms;
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

    // [manifest_cache]
    if (auto* cache = tbl["manifest_cache"].as_table()) {
        if (auto v = (*cache)["restore_on_startup"].value<bool>()) {
            s.manifestRestoreOnStartup = *v;
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
        if (auto v = (*presence)["aetheronline_persona_patch"].value<bool>()) {
            s.presenceAetherOnlinePersonaPatch = *v;
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
        if (auto v = (*presence)["suffix_invisible"].value<bool>()) {
            s.presenceSuffixInvisible = *v;
        }
        if (auto v = (*presence)["appid_blob"].value<bool>()) {
            s.presenceAppIdBlob = *v;
        }
        if (auto* arr = (*presence)["showonline_apps"].as_array()) {
            s.presenceShowOnlineApps.clear();
            for (const auto& item : *arr) {
                if (auto v = item.value<std::int64_t>()) {
                    if (*v > 0) {
                        s.presenceShowOnlineApps.push_back(static_cast<std::uint32_t>(*v));
                    }
                }
            }
        }
        if (const auto* aetherOnlineAppsArr = (*presence)["aetheronline_apps"].as_array()) {
            s.presenceAetherOnlineApps.clear();
            for (const auto& item : *aetherOnlineAppsArr) {
                if (auto v = item.value<std::int64_t>()) {
                    if (*v > 0) {
                        s.presenceAetherOnlineApps.push_back(static_cast<std::uint32_t>(*v));
                    }
                }
            }
        }
        if (auto* arr = (*presence)["exclude_apps"].as_array()) {
            s.presenceExcludeApps.clear();
            for (const auto& item : *arr) {
                if (auto v = item.value<std::int64_t>()) {
                    if (*v > 0) {
                        s.presenceExcludeApps.push_back(static_cast<std::uint32_t>(*v));
                    }
                }
            }
        }
        if (auto v = (*presence)["default_mode"].value<std::string>()) {
            s.presenceDefaultShowOnline = (*v == "showonline");
        }
    }

    AC_LOG_INFO("Settings",
                "Loaded %s (level=%s, keep_last_session=%d, lua extra paths: %zu, "
                "mirror: %s, ost=%d, diversion=%s, manifest urls: %zu, bridge_wait=%dms, manifest_restore=%d, presence: default=%s show=%zu of=%zu excl=%zu).",
                configPath.c_str(),
                s.logLevel == LogLevel::Trace ? "trace"
                    : s.logLevel == LogLevel::Debug ? "debug"
                    : s.logLevel == LogLevel::Info ? "info"
                    : s.logLevel == LogLevel::Warn ? "warn"
                    : s.logLevel == LogLevel::Error ? "error" : "off",
                s.logKeepLastSession ? 1 : 0,
                s.luaExtraPaths.size(),
                s.patternMirror.empty() ? "default" : "custom",
                s.patternUseOstSource ? 1 : 0,
                s.diversionMode == DiversionMode::Auto ? "auto"
                    : s.diversionMode == DiversionMode::Copy ? "copy" : "live",
                s.manifestFetchUrls.size(),
                s.manifestBridgeWaitMs,
                s.manifestRestoreOnStartup ? 1 : 0,
                s.presenceDefaultShowOnline ? "showonline" : "none",
                s.presenceShowOnlineApps.size(),
                s.presenceAetherOnlineApps.size(),
                s.presenceExcludeApps.size());
    if (valid) *valid = true;
    return s;
}

}  // namespace ac
