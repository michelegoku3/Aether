#include "pch.h"
#include "diagnostics/StatusWriter.h"

#include <ctime>
#include "utils/CoalescingWorker.h"
#include <fstream>
#include <string>
#include <vector>

#include "core/AetherCoreState.h"
#include "hooks/aetheronline/PresenceSession.h"
#include "credentials/CredentialStore.h"
#include "network/EticketFetcher.h"
#include "core/HookManager.h"
#include "utils/IpcSpec.h"
#include "core/Logger.h"
#include "utils/JsonWriter.h"
#include "core/AbiSentinel.h"
#include "core/NetPacketAbi.h"
#include "core/StructGuard.h"
#include "scripting/LuaData.h"
#include "hooks/aetheronline/OnlinePayload.h"
#include "hooks/ipc/PipeWatch.h"
#include "hooks/wire/AchievementModule.h"

namespace ac::status {
namespace {

constexpr const char* kModule = "StatusWriter";
// Pinned DLL: intentionally process-lifetime allocation, so CRT detach never
// invokes a joining std::thread destructor. Stop is only called explicitly.
utils::CoalescingWorker* s_worker = nullptr;


bool SaveAtomic(const std::string& path, const std::string& content) {
    const std::string tmp = path + ".tmp";
    {
        std::ofstream out(tmp, std::ios::binary | std::ios::trunc);
        if (!out.is_open()) return false;
        out.write(content.data(), static_cast<std::streamsize>(content.size()));
        out.flush();
        const bool written = out.good();
        out.close();
        if (!written || out.fail()) { DeleteFileA(tmp.c_str()); return false; }
    }
    if (!MoveFileExA(tmp.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING)) {
        DeleteFileA(tmp.c_str());
        return false;
    }
    return true;
}

}  // namespace

static void WriteSnapshot(std::uint64_t requests) {
    // I-C: emissione dichiarativa via jsonw::Writer. La punteggiatura JSON
    // (virgole/indent/newline) vive nel writer, qui c'è solo la lista dei
    // campi. Il formato resta byte-compatibile con schema_version 9: il
    // golden test della suite "jsonwriter" lo fissa carattere per carattere.
    const auto settings = Settings::Snapshot();
    const auto diagnostics = diag::Snapshot();
    const auto hooks = g_state.hookManager.Snapshot();
    const auto& installed = hooks.installed;
    const auto& missed = hooks.missed;
    std::size_t wireEresultEvents = 0;
    std::size_t wireAccessDeniedEvents = 0;
    std::size_t wireTransportCandidateEvents = 0;
    std::size_t cloudBlockedEvents = 0;
    for (const auto& event : diagnostics) {
        if (event.category == "wire_eresult") {
            ++wireEresultEvents;
            if (event.detail.find("label=AccessDenied") != std::string::npos) {
                ++wireAccessDeniedEvents;
            }
            if (event.detail.find("class=transport_or_provider_candidate") != std::string::npos) {
                ++wireTransportCandidateEvents;
            }
        }
        if (event.category == "cloud_state_blocked" || event.category == "cloud_close_blocked") {
            ++cloudBlockedEvents;
        }
    }

    jsonw::Writer w;
    // v4: hooks_missed_list entries carry their reason ("Name (reason)").
    // The field is still a string list, but its CONTENT changed shape, so a
    // consumer that parses hook names has to know (v3 readers keep working).
    w.Int64("schema_version", 9);
    w.Int64("ts", static_cast<long long>(std::time(nullptr)));

    w.Str("build_id", g_state.buildId);
    w.Str("build_config",
#ifdef AETHERCORE_RELEASE
          "Release"
#else
          "Debug"
#endif
    );
    w.Str("build_time", __DATE__ " " __TIME__);
    {
        std::lock_guard lock(g_state.statusMetadataMutex);
        w.Str("diversion_outcome", g_state.diversionOutcome);
        w.Str("steamclient_sha", g_state.steamclientSha);
        w.Bool("steamclient_toml_found", g_state.steamclientTomlFound);
        w.Str("steamclient_pattern_source", g_state.steamclientPatternSource);
        w.Str("steamui_sha", g_state.steamuiSha);
        w.Bool("steamui_toml_found", g_state.steamuiTomlFound);
        w.Str("steamui_pattern_source", g_state.steamuiPatternSource);
    }
    w.Str("steamclient_hook_target", g_state.diversionUsesLive.load() ? "live" : "copy");
    w.Bool("steamui_redirect_installed", g_state.steamUiRedirectInstalled.load());
    w.Bool("steamui_redirect_used", g_state.steamUiRedirectUsed.load());
    w.Str("netpacket_abi_layout", abi::netpkt::LayoutName());
    w.UInt64("netpacket_abi_data_off", abi::netpkt::ResolvedDataOffset());
    w.Bool("netpacket_abi_resolved", abi::netpkt::IsResolved());
    w.Int64("netpacket_abi_probe_attempts", abi::netpkt::ProbeAttempts());
    w.Int64("netpacket_abi_confirmations", abi::netpkt::ProbeConfirmations());
    w.Int64("netpacket_abi_write_rejects", abi::netpkt::WriteRejects());
    w.Str("netpacket_abi_hint_source", abi::netpkt::HintSource());
    w.UInt64("sentinel_verified_count", abi::sentinel::VerifiedCount());
    w.UInt64("sentinel_rejected_count", abi::sentinel::RejectedCount());
    w.UInt64("abi_struct_rejects", abi::guard::RejectionCount());
    w.SizeT("abi_table_fields_checked", abi::guard::TableFieldsChecked());
    w.Bool("abi_layout_contradicted", abi::guard::LayoutContradicted());
    std::size_t benignMisses = 0;
    for (const auto& m : missed) {
        if (IsBenignMiss(m.reason)) ++benignMisses;
    }
    w.SizeT("hooks_installed_count", installed.size());
    w.SizeT("hooks_missed_count", missed.size() - benignMisses);
    w.SizeT("hooks_alias_count", benignMisses);
    w.SizeT("wire_eresult_events", wireEresultEvents);
    w.SizeT("wire_access_denied_events", wireAccessDeniedEvents);
    w.SizeT("wire_transport_candidate_events", wireTransportCandidateEvents);
    w.SizeT("cloud_blocked_events", cloudBlockedEvents);
    w.Bool("package0_captured", g_state.pPackage0.load() != nullptr);
    w.Bool("package0_seeded", g_state.package0Seeded.load());
    w.Bool("config_store_user_local_captured", g_state.pConfigStoreUserLocal.load() != nullptr);
    w.SizeT("config_store_cached_app_tickets", credential::CachedConfigStoreTicketCount());
    w.SizeT("lua_files_loaded", luadata::LoadedFileCount());
    w.SizeT("configured_depots", luadata::ConfiguredDepotCount());
    w.SizeT("access_tokens", luadata::AccessTokenCount());
    w.SizeT("manifest_overrides", luadata::ManifestOverrideCount());
    w.Bool("eticket_backend_configured", !luadata::EticketUrl().empty());
    w.UInt64("eticket_mint_successes", g_state.eticketFetch.mintSuccessCount.load());
    w.UInt64("eticket_mint_failures", g_state.eticketFetch.mintFailureCount.load());
    w.SizeT("eticket_runtime_cache_entries", eticketfetch::CacheCount());
    w.SizeT("eticket_inflight", eticketfetch::InflightCount());
    w.UInt64("ticket_forge_successes", g_state.ticketForgeSuccessCount.load());
    w.UInt64("ticket_forge_failures", g_state.ticketForgeFailureCount.load());
    w.Str("manifest_fetch_production_route", "authenticated_hubcap_standalone");
    w.Bool("online_payload_present",
           GetFileAttributesA(g_state.payloadDllPath.c_str()) != INVALID_FILE_ATTRIBUTES);
    w.SizeT("online_payload_injected_pids", hooks::onlinepayload::InjectedPidCount());
    w.UInt64("online_payload_inject_successes", g_state.onlinePayload.injectSuccessCount.load());
    w.UInt64("online_payload_inject_failures", g_state.onlinePayload.injectFailureCount.load());
    w.SizeT("pipewatch_snapshots", pipewatch::SnapshotCount());
    w.SizeT("pipewatch_evictions", pipewatch::EvictionCount());
    // Do not read the maps while the init/retry thread is publishing them.
    const bool ipcLoaded = g_state.ipcSpec.loaded.load();
    std::size_t ipcEntries = 0, withFencepost = 0, withArgc = 0;
    if (ipcLoaded) {
        ipcEntries = g_state.ipcSpec.methods.size();
        for (const auto& [_, spec] : g_state.ipcSpec.methods) {
            if (spec.fencepost != 0) ++withFencepost;
            if (spec.argc != 0) ++withArgc;
        }
    }
    w.Bool("ipc_spec_loaded", ipcLoaded);
    w.SizeT("ipc_spec_entries", ipcEntries);
    w.SizeT("ipc_spec_methods_with_fencepost", withFencepost);
    w.SizeT("ipc_spec_methods_with_argc", withArgc);
    {
        std::lock_guard<std::mutex> lock(g_state.presence.mutex);
        w.UInt64("presence_playing_appid", g_state.presence.playingAppId);
        w.UInt64("presence_self_steamid", g_state.presence.selfSteamId);
        w.Bool("presence_have_template", g_state.presence.haveSelfTemplate);
        w.Bool("presence_inject_pending", g_state.presence.injectPending);
        w.UInt64("presence_inject_deliveries", g_state.presence.injectDeliverCount);
        w.UInt64("presence_inject_build_fails", g_state.presence.injectBuildFailCount);
        w.UInt64("presence_gamesplayed_tracks", g_state.presence.gamesPlayedTrackCount);
        w.UInt64("presence_extra_info_patches", g_state.presence.extraInfoPatchCount);
    }
    w.Bool("presence_inject_local", settings->presenceInjectLocal);
    w.Bool("presence_always_extra_info", settings->presenceAlwaysExtraInfo);
    w.Bool("presence_showonline_broadcast", settings->presenceShowOnlineBroadcast);
    w.Bool("presence_friend_appid_from_name", settings->presenceFriendAppIdFromName);
    w.UInt64("aetheronline_real_appid", presence::RealAppId());
    w.UInt64("showonline_appid", presence::ShowOnlineAppId());
    w.UInt64("license_reload_forced_count", g_state.licenseReloadForcedCount.load());
    w.UInt64("license_reload_direct_count", g_state.licenseReloadDirectCount.load());
    w.SizeT("gamename_cache_size", g_state.gameName.nameCache.Size());
    w.SizeT("gamename_cache_hits", g_state.gameName.nameCache.HitCount());
    w.SizeT("gamename_cache_misses", g_state.gameName.nameCache.MissCount());
    w.SizeT("gamename_cache_evictions", g_state.gameName.nameCache.EvictionCount());
    w.SizeT("gamename_cache_negative", g_state.gameName.nameCache.NegativeCount());

    w.StrArray("hooks_installed_list", installed);

    // Each entry carries its reason ("Name (address collision with another
    // hook)"): the status file is the only place a hook miss survives after
    // the log rotates, and "missed" alone could not distinguish a pattern
    // that the build does not have from a hook deliberately not applied.
    std::vector<std::string> missedTexts;
    missedTexts.reserve(missed.size());
    for (const auto& m : missed) {
        missedTexts.push_back(MissedHookText(m.name, m.reason, m.detail));
    }
    w.StrArray("hooks_missed_list", missedTexts);

    w.ObjectArray("diagnostics");
    for (const auto& d : diagnostics) {
        w.RawObject("{\"ts_ms\": " + std::to_string(d.timestampMs) +
                    ", \"category\": \"" + jsonw::Escape(d.category) +
                    "\", \"detail\": \"" + jsonw::Escape(d.detail) + "\"}");
    }
    w.ArrayEnd();

    const std::string path = g_state.aetherCoreDir + "\\status.json";
    if (SaveAtomic(path, w.Finish())) {
        AC_LOG_DEBUG(kModule, "Wrote %s (installed=%zu, missed=%zu, coalesced_requests=%llu).",
                    path.c_str(), installed.size(), missed.size(),
                    static_cast<unsigned long long>(requests));
    } else {
        AC_LOG_WARN(kModule, "Failed to write %s.", path.c_str());
    }
}

void Start() {
    if (s_worker) return;
    s_worker = new utils::CoalescingWorker;
    s_worker->Start([](std::uint64_t requests) {
        try { WriteSnapshot(requests); }
        catch (const std::exception& e) { AC_LOG_WARN(kModule, "Snapshot failed: %s", e.what()); }
        catch (...) { AC_LOG_WARN(kModule, "Snapshot failed: unknown exception."); }
    });
    AC_LOG_INFO(kModule, "Async writer started (100 ms coalescing). Success details at DEBUG.");
}

void Write() {
    if (s_worker && !g_state.shuttingDown.load()) s_worker->Request();
}

void Stop() {
    if (s_worker) s_worker->Stop();
    AC_LOG_INFO(kModule, "Async writer stopped.");
}

}  // namespace ac::status
