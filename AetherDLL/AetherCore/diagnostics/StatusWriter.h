#pragma once

// ---------------------------------------------------------------------------
// Writes <Steam>\aethercore\status.json so an external GUI can show whether
// the running Steam build is supported (pattern available + hooks installed).
//
// LumaCore shipped two overlapping status systems (DOCS_TODO 13 #11);
// AetherCore has exactly one. It reads everything it needs from g_state and
// HookManager, so there is no separate counter bookkeeping to keep in sync.
//
// Schema v8 (top-level keys include):
//   schema_version, ts
//   build_id, build_config, build_time, diversion_outcome
//   steamclient_sha, steamclient_toml_found, steamclient_pattern_source
//   steamui_sha, steamui_toml_found, steamui_pattern_source
//   netpacket_abi_layout, netpacket_abi_data_off, netpacket_abi_resolved,
//     netpacket_abi_probe_attempts, netpacket_abi_confirmations,
//     netpacket_abi_write_rejects, netpacket_abi_hint_source
//   sentinel_verified_count, sentinel_rejected_count, abi_struct_rejects
//   hooks_installed_count, hooks_missed_count, hooks_alias_count
//   wire_eresult_events, wire_access_denied_events, wire_transport_candidate_events
//   cloud_blocked_events
//   package0_captured, package0_seeded
//   config_store_user_local_captured, config_store_cached_app_tickets
//   lua_files_loaded, configured_depots, access_tokens, manifest_overrides
//   eticket_backend_configured, eticket_mint_successes, eticket_mint_failures,
//     eticket_runtime_cache_entries
//   ticket_forge_successes, ticket_forge_failures
//   manifest_fetch_production_route (local | authenticated_hubcap_standalone | legacy_provider)
//   online_payload_present, online_payload_injected_pids,
//     online_payload_inject_successes, online_payload_inject_failures
//   pipewatch_snapshots
//   ipc_spec_loaded, ipc_spec_entries
//   hooks_installed_list[], hooks_missed_list[] ("Name (reason)"), diagnostics[]
//
// History: v8 added netpacket_abi_hint_source (which source proposed the
// layout the probe then confirmed: the per-build ABI table, the compiled
// build hints, or nothing at all);
// v7 added abi_struct_rejects (PackageInfo / AppOwnership writes
// refused by the struct guards);
// v6 added netpacket_abi_confirmations / netpacket_abi_write_rejects
// (ABI write barrier) and sentinel_verified_count / sentinel_rejected_count
// (function-entry verification performed before any hook is installed);
// v5 added the netpacket_abi_* keys (per-build CNetPacket layout
// state: which layout was identified, from how many probe attempts, or
// whether the wire features are disabled because it could not be) and
// hooks_alias_count (hooks_missed_count no longer counts known aliases such
// as ConfigStoreGetBinary == LoadDepotDecryptionKey; those stay listed in
// hooks_missed_list with their reason);
// v4 changed hooks_missed_list entries from "Name" to
// "Name (reason)" (the reason is otherwise lost when the log rotates);
// v3 was the first AetherCore schema.
// ---------------------------------------------------------------------------
namespace ac::status {

// Lifecycle functions: initialization owner only, never under loader lock.
void Start();
void Stop();

// Requests a write (atomic counter only). One worker coalesces requests at
// 100 ms intervals, takes snapshots and writes atomically. Best-effort.
void Write();

}  // namespace ac::status
