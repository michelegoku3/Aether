//! Tests for FE <-> BE bridge contracts, IPC command schemas, and serde serialization.
//!
//! Validates that backend structures serialize with the exact field names
//! and shapes expected by the frontend TypeScript consumers, and that critical command
//! identifiers are accounted for.

use serde_json::Value;

use crate::manifest::pins::LuaManifestRow;
use crate::providers::hubcap::{HubcapGenerationBucket, HubcapGenerationUsage};
use crate::steamless::runner::SteamlessRunResult;
use crate::updater::github::ComponentUpdateInfo;

#[test]
fn test_component_update_info_contract() {
    let info = ComponentUpdateInfo {
        installed_version: "1.4.0".to_string(),
        latest_version: "1.4.1".to_string(),
        latest_tag: "desk-1.4.1".to_string(),
        update_available: true,
        is_test: false,
        release_url: "https://github.com/example/release".to_string(),
        notes: "Release notes".to_string(),
    };

    let serialized = serde_json::to_value(&info).expect("serialize ComponentUpdateInfo");
    assert!(serialized.is_object());

    // Frontend App.tsx DeskUpdateInfo expects snake_case properties:
    assert_eq!(serialized.get("installed_version"), Some(&Value::String("1.4.0".to_string())));
    assert_eq!(serialized.get("latest_version"), Some(&Value::String("1.4.1".to_string())));
    assert_eq!(serialized.get("latest_tag"), Some(&Value::String("desk-1.4.1".to_string())));
    assert_eq!(serialized.get("update_available"), Some(&Value::Bool(true)));
    assert_eq!(serialized.get("is_test"), Some(&Value::Bool(false)));
    assert_eq!(serialized.get("release_url"), Some(&Value::String("https://github.com/example/release".to_string())));
    assert_eq!(serialized.get("notes"), Some(&Value::String("Release notes".to_string())));
}

#[test]
fn test_steamless_run_result_camel_case_contract() {
    let result = SteamlessRunResult {
        success: true,
        cancelled: false,
        message: "Unpacked successfully".to_string(),
        exe_path: Some("C:\\Games\\game.exe".to_string()),
        backup_path: Some("C:\\Games\\game.exe.bak".to_string()),
        stdout_tail: "stdout".to_string(),
        stderr_tail: "stderr".to_string(),
    };

    let serialized = serde_json::to_value(&result).expect("serialize SteamlessRunResult");
    assert_eq!(serialized.get("success"), Some(&Value::Bool(true)));
    assert_eq!(serialized.get("cancelled"), Some(&Value::Bool(false)));
    assert!(serialized.get("exePath").is_some());
    assert!(serialized.get("backupPath").is_some());
    assert!(serialized.get("stdoutTail").is_some());
    assert!(serialized.get("stderrTail").is_some());

    assert!(serialized.get("exe_path").is_none());
    assert!(serialized.get("backup_path").is_none());
    assert!(serialized.get("stdout_tail").is_none());
}

#[test]
fn test_hubcap_generation_usage_serialization() {
    let usage = HubcapGenerationUsage {
        single: HubcapGenerationBucket {
            usage: Some(5),
            limit: Some(50),
            remaining: Some(45),
        },
        bundle: HubcapGenerationBucket::default(),
        workshop: HubcapGenerationBucket {
            usage: Some(0),
            limit: Some(10),
            remaining: Some(10),
        },
        steam_service_ready: Some(true),
    };

    let serialized = serde_json::to_value(&usage).expect("serialize HubcapGenerationUsage");
    assert!(serialized.get("single").is_some());
    assert!(serialized.get("bundle").is_some());
    assert!(serialized.get("workshop").is_some());
    assert_eq!(serialized.get("steam_service_ready"), Some(&Value::Bool(true)));
}

#[test]
fn test_lua_manifest_row_camel_case_contract() {
    let row = LuaManifestRow {
        row_id: 1,
        app_id: 12345,
        manifest_id: "987654321012345678".to_string(),
        enabled: true,
        issue: None,
    };

    let serialized = serde_json::to_value(&row).expect("serialize LuaManifestRow");
    // LuaManifestRow uses #[serde(rename_all = "camelCase")]
    assert_eq!(serialized.get("rowId"), Some(&Value::Number(1.into())));
    assert_eq!(serialized.get("appId"), Some(&Value::Number(12345.into())));
    assert_eq!(
        serialized.get("manifestId"),
        Some(&Value::String("987654321012345678".to_string()))
    );
    assert_eq!(serialized.get("enabled"), Some(&Value::Bool(true)));
    assert!(serialized.get("issue").is_none());
}

// The old `test_core_commands_inventory_check` was removed: it listed command
// names and asserted they were not empty strings — true by construction, hence
// unable to flag any drift (it even contained `save_installed_lua_manifest_rows`,
// a command that never existed).
//
// The FE <-> BE contract is now verified against the real sources in
// `src/tests/ipc_contract_tests.rs`: registered vs defined commands, frontend-
// invoked names vs registered ones, argument keys vs Rust signatures, wire key
// shape (the `showonline` case), no `steam_path` in payloads, and an explicit
// list of unused commands.

// ---------------------------------------------------------------------------
// RESPONSE contract: synchronizer snapshot <-> src/types/sync.ts
// ---------------------------------------------------------------------------
//
// The argument contract test (`ipc_contract_tests.rs`) covers the
// frontend -> backend direction. Here is the other one: the keys the backend
// SERIALIZES must match the TypeScript interface reading them.
//
// Same bug class with the same silent outcome: `MonitorStatus` declares
// `pinSync`, Rust serializes `pin_sync`, and the popup shows a perpetually
// empty lane instead of an error. The case is concrete because the snapshot
// travels both as the `get_hubcap_monitor_status` response and as the
// `sync://status-changed` push event payload.

use std::collections::BTreeSet;

use crate::core::hubcap_update_monitor::{LaneStatus, MonitorStatusSnapshot, PendingTaskInfo};

/// Chiavi di primo livello di un oggetto JSON.
fn json_object_keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .expect("the serialized value must be an object")
        .keys()
        .cloned()
        .collect()
}

/// Keys declared by `export interface <name> { … }` in a TS file.
///
/// Deliberately simple reading: one key per line, shaped `name?: type;` —
/// that is how this project's mirror interfaces are written. When the format
/// changes, the test says so instead of silently passing (no keys found =
/// failure).
fn ts_interface_keys(source: &str, name: &str) -> BTreeSet<String> {
    let header = format!("export interface {name} {{");
    let start = source
        .find(&header)
        .unwrap_or_else(|| panic!("interface `{name}` not found in the TS file"));
    let body_start = start + header.len();
    let mut depth = 1usize;
    let mut end = body_start;
    for (offset, ch) in source[body_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = body_start + offset;
                    break;
                }
            }
            _ => {}
        }
    }
    let mut keys = BTreeSet::new();
    for line in source[body_start..end].lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("/*") || line.starts_with('*') || line.starts_with("//") {
            continue;
        }
        let Some(colon) = line.find(':') else { continue };
        let key = line[..colon].trim().trim_end_matches('?').trim();
        if key.is_empty() || !key.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
            continue;
        }
        keys.insert(key.to_string());
    }
    assert!(
        !keys.is_empty(),
        "nessuna chiave estratta dall'interfaccia TS `{name}`: il parser del test va aggiornato"
    );
    keys
}

fn types_sync_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("AetherDesk/ above src-tauri/")
        .join("src")
        .join("types")
        .join("sync.ts");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("impossibile leggere {}: {error}", path.display()))
}

fn sample_snapshot() -> MonitorStatusSnapshot {
    MonitorStatusSnapshot {
        running: true,
        started_epoch: Some(1_700_000_000),
        last_scan_epoch: Some(1_700_000_020),
        steam_path_configured: true,
        hubcap_key_configured: false,
        checkpoint_initialized: true,
        pin_sync: LaneStatus {
            pending: vec![PendingTaskInfo {
                app_id: Some(480),
                attempts: 1,
                next_retry_epoch: Some(1_700_000_080),
            }],
            unresolved: vec![PendingTaskInfo {
                app_id: Some(440),
                attempts: 5,
                next_retry_epoch: None,
            }],
            processed_count: 3,
            last_run_epoch: Some(1_700_000_010),
            last_error: Some("manifest not found".to_string()),
        },
        ..MonitorStatusSnapshot::default()
    }
}

#[test]
fn monitor_status_snapshot_matches_the_typescript_mirror() {
    let source = types_sync_source();
    let json = serde_json::to_value(sample_snapshot()).expect("serialize MonitorStatusSnapshot");

    let rust_top = json_object_keys(&json);
    let ts_top = ts_interface_keys(&source, "MonitorStatus");
    assert_eq!(
        rust_top, ts_top,
        "\nSnapshot keys diverged between Rust and TypeScript.\n  Rust only: {:?}\n  TS only:   {:?}\n\
         Align src/types/sync.ts or #[serde(rename_all)] in core/hubcap_update_monitor.rs.\n",
        rust_top.difference(&ts_top).collect::<Vec<_>>(),
        ts_top.difference(&rust_top).collect::<Vec<_>>(),
    );

    let lane = json
        .get("pinSync")
        .expect("the pinSync lane must serialize in camelCase");
    let rust_lane = json_object_keys(lane);
    let ts_lane = ts_interface_keys(&source, "LaneStatus");
    assert_eq!(
        rust_lane, ts_lane,
        "\nLaneStatus keys diverged.\n  Rust only: {:?}\n  TS only:   {:?}\n",
        rust_lane.difference(&ts_lane).collect::<Vec<_>>(),
        ts_lane.difference(&rust_lane).collect::<Vec<_>>(),
    );

    let task = lane
        .get("pending")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .expect("pending must hold at least one task in the sample");
    let rust_task = json_object_keys(task);
    let ts_task = ts_interface_keys(&source, "PendingTaskInfo");
    assert_eq!(
        rust_task, ts_task,
        "\nPendingTaskInfo keys diverged.\n  Rust only: {:?}\n  TS only:   {:?}\n",
        rust_task.difference(&ts_task).collect::<Vec<_>>(),
        ts_task.difference(&rust_task).collect::<Vec<_>>(),
    );
}

#[test]
fn monitor_status_snapshot_serializes_every_field_even_when_empty() {
    // The popup reads the four lanes with no presence checks: if an `Option`
    // field were omitted when `None` (skip_serializing_if), the UI would get
    // `undefined` where it expects a number or an array.
    let json = serde_json::to_value(MonitorStatusSnapshot::default()).expect("serialize default");
    let keys = json_object_keys(&json);
    for expected in [
        "running",
        "startedEpoch",
        "lastScanEpoch",
        "steamPathConfigured",
        "hubcapKeyConfigured",
        "checkpointInitialized",
        "pinSync",
        "pinRefresh",
        "repair",
        "workshop",
    ] {
        assert!(
            keys.contains(expected),
            "lo snapshot vuoto deve comunque esporre `{expected}` (chiavi: {keys:?})"
        );
    }
    let lane = json.get("workshop").expect("corsia workshop");
    for expected in ["pending", "unresolved", "processedCount", "lastRunEpoch", "lastError"] {
        assert!(
            json_object_keys(lane).contains(expected),
            "la corsia vuota deve comunque esporre `{expected}`"
        );
    }
}
