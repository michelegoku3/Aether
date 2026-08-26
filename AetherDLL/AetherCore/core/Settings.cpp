#include "pch.h"
#include "core/Settings.h"

#include <atomic>
#include <chrono>
#include <filesystem>
#include <toml++/toml.hpp>
#include "core/AetherCoreState.h"

namespace ac {

namespace {
// Mtime of the config as of the last successful Load() attempt, so
// ReloadIfModified can compare against what is ACTUALLY in memory instead of
// initialising its own clock on first call (which silently skipped the first
// real change: one launch after a Desk edit resolved against stale settings).
std::atomic<long long> s_lastConfigWriteTicks{0};

long long FileWriteTicks(const std::string& configPath) {
    std::error_code ec;
    const auto t = std::filesystem::last_write_time(configPath, ec);
    if (ec) return 0;
    return std::chrono::duration_cast<std::chrono::milliseconds>(t.time_since_epoch()).count();
}
}  // namespace

void Settings::ReloadIfModified(const std::string& configPath) {
    if (configPath.empty()) return;
    const long long ticks = FileWriteTicks(configPath);
    if (ticks == 0) return;
    long long prev = s_lastConfigWriteTicks.load();
    if (ticks == prev) return;
    if (s_lastConfigWriteTicks.compare_exchange_strong(prev, ticks)) {
        g_state.settings = Settings::Load(configPath);
        // The logger level lives in the config too: re-apply it so a Desk-side
        // level change takes effect WITHOUT restarting Steam. ReloadIfModified
        // runs on every game launch, so the worst-case delay is one launch.
        ac::log::SetLevel(g_state.settings.logLevel);
        AC_LOG_INFO("Settings", "Hot-reloaded settings from %s (custom_game_name='%s').",
                    configPath.c_str(), g_state.settings.presenceCustomGameName.c_str());
    }
}

Settings Settings::Load(const std::string& configPath) {
    Settings s;  // Start from defaults; only override what the file provides.

    // Record the mtime we are loading, so the next ReloadIfModified sees a
    // genuine change instead of treating the first call as a warm-up.
    {
        const long long ticks = FileWriteTicks(configPath);
        if (ticks != 0) s_lastConfigWriteTicks.store(ticks);
    }

    toml::table tbl;
    try {
        tbl = toml::parse_file(configPath);
    } catch (const toml::parse_error& e) {
        // WARN on purpose: a broken config falls back to ALL defaults (log
        // level included: Warn in release builds), so an INFO here would be
        // filtered out by the very default it caused — a silent-failure loop
        // that leaves presence lists empty with zero evidence in main.log.
        AC_LOG_WARN("Settings",
                    "Config %s is invalid TOML (%s) — using defaults (policies OFF, log level forced to default).",
                    configPath.c_str(), e.description());
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
        // aetheronline_persona_patch (legacy key: onlinefix_persona_patch).
        if (auto v = (*presence)["aetheronline_persona_patch"].value<bool>()) {
            s.presenceAetherOnlinePersonaPatch = *v;
        } else if (auto v = (*presence)["onlinefix_persona_patch"].value<bool>()) {
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
        // aetheronline_apps (legacy key: onlinefix_apps — pre-rename configs).
        const auto* aetherOnlineAppsArr = (*presence)["aetheronline_apps"].as_array();
        if (!aetherOnlineAppsArr) {
            aetherOnlineAppsArr = (*presence)["onlinefix_apps"].as_array();
        }
        if (aetherOnlineAppsArr) {
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
                "mirror: %s, manifest urls: %zu, presence: default=%s show=%zu of=%zu excl=%zu).",
                configPath.c_str(),
                s.logLevel == LogLevel::Trace ? "trace"
                    : s.logLevel == LogLevel::Debug ? "debug"
                    : s.logLevel == LogLevel::Info ? "info"
                    : s.logLevel == LogLevel::Warn ? "warn"
                    : s.logLevel == LogLevel::Error ? "error" : "off",
                s.logKeepLastSession ? 1 : 0,
                s.luaExtraPaths.size(),
                s.patternMirror.empty() ? "default" : "custom",
                s.manifestFetchUrls.size(),
                s.presenceDefaultShowOnline ? "showonline" : "none",
                s.presenceShowOnlineApps.size(),
                s.presenceAetherOnlineApps.size(),
                s.presenceExcludeApps.size());
    return s;
}

}  // namespace ac
