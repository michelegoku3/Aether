#include "pch.h"
#include "diagnostics/StatusWriter.h"

#include <ctime>
#include <fstream>
#include <sstream>
#include <string>

#include "core/AetherCoreState.h"
#include "credentials/CredentialStore.h"
#include "network/EticketFetcher.h"
#include "core/HookManager.h"
#include "utils/IpcSpec.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "network/ManifestFetch.h"
#include "hooks/onlinefix/OnlinePayload.h"
#include "hooks/ipc/PipeWatch.h"
#include "hooks/wire/AchievementModule.h"

namespace ac::status {
namespace {

constexpr const char* kModule = "StatusWriter";

// Minimal JSON string escaping. Hook names and SHAs are ASCII, but escaping
// quotes/backslashes keeps the output valid for any input.
std::string EscapeJson(const std::string& in) {
    std::string out;
    out.reserve(in.size() + 2);
    for (char c : in) {
        switch (c) {
            case '"':  out += "\\\""; break;
            case '\\': out += "\\\\"; break;
            case '\n': out += "\\n";  break;
            case '\r': out += "\\r";  break;
            case '\t': out += "\\t";  break;
            default:   out += c;      break;
        }
    }
    return out;
}

bool SaveAtomic(const std::string& path, const std::string& content) {
    const std::string tmp = path + ".tmp";
    {
        std::ofstream out(tmp, std::ios::binary | std::ios::trunc);
        if (!out.is_open()) return false;
        out.write(content.data(), static_cast<std::streamsize>(content.size()));
    }
    if (!MoveFileExA(tmp.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING)) {
        DeleteFileA(tmp.c_str());
        return false;
    }
    return true;
}

}  // namespace

void Write() {
    std::ostringstream json;
    const auto diagnostics = diag::Snapshot();
    const auto& installed = g_state.hookManager.InstalledHooks();
    const auto& missed = g_state.hookManager.MissedHooks();

    json << "{\n";
    json << "  \"schema_version\": 2,\n";
    json << "  \"ts\": " << static_cast<long long>(std::time(nullptr)) << ",\n";
    json << "  \"build_id\": \"" << EscapeJson(g_state.buildId) << "\",\n";
    json << "  \"build_config\": \""
#ifdef AETHERCORE_RELEASE
         << "Release"
#else
         << "Debug"
#endif
         << "\",\n";
    json << "  \"build_time\": \"" << __DATE__ << " " << __TIME__ << "\",\n";
    json << "  \"diversion_outcome\": \"" << EscapeJson(g_state.diversionOutcome) << "\",\n";
    json << "  \"steamclient_sha\": \"" << EscapeJson(g_state.steamclientSha) << "\",\n";
    json << "  \"steamclient_toml_found\": " << (g_state.steamclientTomlFound ? "true" : "false") << ",\n";
    json << "  \"steamclient_pattern_source\": \"" << EscapeJson(g_state.steamclientPatternSource) << "\",\n";
    json << "  \"steamui_sha\": \"" << EscapeJson(g_state.steamuiSha) << "\",\n";
    json << "  \"steamui_toml_found\": " << (g_state.steamuiTomlFound ? "true" : "false") << ",\n";
    json << "  \"steamui_pattern_source\": \"" << EscapeJson(g_state.steamuiPatternSource) << "\",\n";
    json << "  \"hooks_installed_count\": " << installed.size() << ",\n";
    json << "  \"hooks_missed_count\": " << missed.size() << ",\n";
    json << "  \"package0_captured\": " << (g_state.pPackage0.load() ? "true" : "false") << ",\n";
    json << "  \"package0_seeded\": " << (g_state.package0Seeded.load() ? "true" : "false") << ",\n";
    json << "  \"config_store_user_local_captured\": "
         << (g_state.pConfigStoreUserLocal.load() ? "true" : "false") << ",\n";
    json << "  \"config_store_cached_app_tickets\": " << credential::CachedConfigStoreTicketCount() << ",\n";
    json << "  \"lua_files_loaded\": " << luadata::LoadedFileCount() << ",\n";
    json << "  \"configured_depots\": " << luadata::ConfiguredDepotCount() << ",\n";
    json << "  \"access_tokens\": " << luadata::AccessTokenCount() << ",\n";
    json << "  \"manifest_overrides\": " << luadata::ManifestOverrideCount() << ",\n";
    json << "  \"eticket_backend_configured\": " << (!luadata::EticketUrl().empty() ? "true" : "false") << ",\n";
    json << "  \"eticket_mint_successes\": " << g_state.eticketFetch.mintSuccessCount.load() << ",\n";
    json << "  \"eticket_mint_failures\": " << g_state.eticketFetch.mintFailureCount.load() << ",\n";
    json << "  \"eticket_runtime_cache_entries\": " << eticketfetch::CacheCount() << ",\n";
    json << "  \"eticket_inflight\": " << eticketfetch::InflightCount() << ",\n";
    json << "  \"achievement_donor_pending\": " << hooks::AchievementModule::PendingDonorResolves() << ",\n";
    json << "  \"achievement_donor_cache_size\": " << g_state.achievements.apiCache.Size() << ",\n";
    json << "  \"achievement_donor_cache_hits\": " << g_state.achievements.apiCache.HitCount() << ",\n";
    json << "  \"achievement_donor_cache_misses\": " << g_state.achievements.apiCache.MissCount() << ",\n";
    json << "  \"achievement_donor_cache_evictions\": " << g_state.achievements.apiCache.EvictionCount() << ",\n";
    json << "  \"achievement_donor_cache_negative\": " << g_state.achievements.apiCache.NegativeCount() << ",\n";
    json << "  \"ticket_forge_successes\": " << g_state.ticketForgeSuccessCount.load() << ",\n";
    json << "  \"ticket_forge_failures\": " << g_state.ticketForgeFailureCount.load() << ",\n";
    json << "  \"manifest_fetch_pending\": " << manifestfetch::PendingCount() << ",\n";
    json << "  \"manifest_fetch_cache_entries\": " << manifestfetch::CacheCount() << ",\n";
    json << "  \"online_payload_present\": "
         << (GetFileAttributesA(g_state.payloadDllPath.c_str()) != INVALID_FILE_ATTRIBUTES ? "true" : "false") << ",\n";
    json << "  \"online_payload_injected_pids\": " << hooks::onlinepayload::InjectedPidCount() << ",\n";
    json << "  \"online_payload_inject_successes\": " << g_state.onlinePayload.injectSuccessCount.load() << ",\n";
    json << "  \"online_payload_inject_failures\": " << g_state.onlinePayload.injectFailureCount.load() << ",\n";
    json << "  \"pipewatch_snapshots\": " << pipewatch::SnapshotCount() << ",\n";
    json << "  \"ipc_spec_loaded\": " << (g_state.ipcSpec.loaded ? "true" : "false") << ",\n";
    json << "  \"ipc_spec_entries\": " << g_state.ipcSpec.methods.size() << ",\n";
    {
        std::size_t withFencepost = 0;
        std::size_t withArgc = 0;
        for (const auto& [_, spec] : g_state.ipcSpec.methods) {
            if (spec.fencepost != 0) ++withFencepost;
            if (spec.argc != 0) ++withArgc;
        }
        json << "  \"ipc_spec_methods_with_fencepost\": " << withFencepost << ",\n";
        json << "  \"ipc_spec_methods_with_argc\": " << withArgc << ",\n";
    }
    {
        std::lock_guard<std::mutex> lock(g_state.presence.mutex);
        json << "  \"presence_playing_appid\": " << g_state.presence.playingAppId << ",\n";
        json << "  \"presence_self_steamid\": " << g_state.presence.selfSteamId << ",\n";
        json << "  \"presence_have_template\": "
             << (g_state.presence.haveSelfTemplate ? "true" : "false") << ",\n";
        json << "  \"presence_inject_pending\": "
             << (g_state.presence.injectPending ? "true" : "false") << ",\n";
        json << "  \"presence_inject_deliveries\": " << g_state.presence.injectDeliverCount
             << ",\n";
        json << "  \"presence_inject_build_fails\": " << g_state.presence.injectBuildFailCount
             << ",\n";
        json << "  \"presence_gamesplayed_tracks\": " << g_state.presence.gamesPlayedTrackCount
             << ",\n";
        json << "  \"presence_extra_info_patches\": " << g_state.presence.extraInfoPatchCount
             << ",\n";
    }
    json << "  \"presence_inject_local\": "
         << (g_state.settings.presenceInjectLocal ? "true" : "false") << ",\n";
    json << "  \"presence_always_extra_info\": "
         << (g_state.settings.presenceAlwaysExtraInfo ? "true" : "false") << ",\n";
    json << "  \"onlinefix_real_appid\": " << g_state.onlineFixRealAppId.load() << ",\n";
    json << "  \"license_reload_forced_count\": " << g_state.licenseReloadForcedCount.load() << ",\n";
    json << "  \"license_reload_direct_count\": " << g_state.licenseReloadDirectCount.load() << ",\n";
    json << "  \"gamename_cache_size\": " << g_state.gameName.nameCache.Size() << ",\n";
    json << "  \"gamename_cache_hits\": " << g_state.gameName.nameCache.HitCount() << ",\n";
    json << "  \"gamename_cache_misses\": " << g_state.gameName.nameCache.MissCount() << ",\n";
    json << "  \"gamename_cache_evictions\": " << g_state.gameName.nameCache.EvictionCount() << ",\n";
    json << "  \"gamename_cache_negative\": " << g_state.gameName.nameCache.NegativeCount() << ",\n";

    json << "  \"hooks_installed_list\": [";
    for (std::size_t i = 0; i < installed.size(); ++i) {
        json << (i == 0 ? "\n    " : ",\n    ") << '"' << EscapeJson(installed[i]) << '"';
    }
    json << (installed.empty() ? "],\n" : "\n  ],\n");

    json << "  \"hooks_missed_list\": [";
    for (std::size_t i = 0; i < missed.size(); ++i) {
        json << (i == 0 ? "\n    " : ",\n    ") << '"' << EscapeJson(missed[i]) << '"';
    }
    json << (missed.empty() ? "],\n" : "\n  ],\n");

    json << "  \"diagnostics\": [";
    for (std::size_t i = 0; i < diagnostics.size(); ++i) {
        const auto& d = diagnostics[i];
        json << (i == 0 ? "\n    " : ",\n    ")
             << "{\"ts_ms\": " << d.timestampMs
             << ", \"category\": \"" << EscapeJson(d.category)
             << "\", \"detail\": \"" << EscapeJson(d.detail) << "\"}";
    }
    json << (diagnostics.empty() ? "]\n" : "\n  ]\n");
    json << "}\n";

    const std::string path = g_state.aetherCoreDir + "\\status.json";
    if (SaveAtomic(path, json.str())) {
        AC_LOG_INFO(kModule, "Wrote %s (installed=%zu, missed=%zu).",
                    path.c_str(), installed.size(), missed.size());
    } else {
        AC_LOG_WARN(kModule, "Failed to write %s.", path.c_str());
    }
}

}  // namespace ac::status
