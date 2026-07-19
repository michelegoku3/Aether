#pragma once

// ---------------------------------------------------------------------------
// Writes <Steam>\aethercore\status.json so an external GUI can show whether
// the running Steam build is supported (pattern available + hooks installed).
//
// LumaCore shipped two overlapping status systems (DOCS_TODO 13 #11);
// AetherCore has exactly one. It reads everything it needs from g_state and
// HookManager, so there is no separate counter bookkeeping to keep in sync.
//
// Schema v2 (top-level keys include):
//   schema_version, ts
//   build_id, build_config, build_time, diversion_outcome
//   steamclient_sha, steamclient_toml_found, steamclient_pattern_source
//   steamui_sha, steamui_toml_found, steamui_pattern_source
//   hooks_installed_count, hooks_missed_count
//   package0_captured, package0_seeded
//   config_store_user_local_captured, config_store_cached_app_tickets
//   lua_files_loaded, configured_depots, access_tokens, manifest_overrides
//   eticket_backend_configured, eticket_mint_successes, eticket_mint_failures,
//     eticket_runtime_cache_entries
//   ticket_forge_successes, ticket_forge_failures
//   manifest_fetch_pending, manifest_fetch_cache_entries
//   online_payload_present, online_payload_injected_pids,
//     online_payload_inject_successes, online_payload_inject_failures
//   pipewatch_snapshots
//   ipc_spec_loaded, ipc_spec_entries
//   hooks_installed_list[], hooks_missed_list[], diagnostics[]
// ---------------------------------------------------------------------------
namespace ac::status {

// Serialises the current snapshot and writes it atomically. Best-effort:
// failures are logged, never thrown.
void Write();

}  // namespace ac::status
