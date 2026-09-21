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

// Il vecchio `test_core_commands_inventory_check` è stato rimosso: elencava nomi
// di comandi e assertiva che non fossero stringhe vuote — vero per
// costruzione, quindi incapace di segnalare qualunque deriva (conteneva anche
// `save_installed_lua_manifest_rows`, comando che non è mai esistito).
//
// Il contratto FE <-> BE è ora verificato sul sorgente reale in
// `src/tests/ipc_contract_tests.rs`: comandi registrati vs definiti, nomi
// invocati dal frontend vs registrati, chiavi degli argomenti vs firme Rust,
// forma delle chiavi wire (il caso `showonline`), assenza di `steam_path` nei
// payload e lista esplicita dei comandi inutilizzati.

// ---------------------------------------------------------------------------
// Contratto di RISPOSTA: snapshot del sincronizzatore <-> src/types/sync.ts
// ---------------------------------------------------------------------------
//
// Il test di contratto sugli argomenti (`ipc_contract_tests.rs`) copre la
// direzione frontend -> backend. Qui c'è l'altra: le chiavi che il backend
// SERIALIZZA devono coincidere con l'interfaccia TypeScript che le legge.
//
// È la stessa classe di bug con lo stesso esito silenzioso: `MonitorStatus`
// dichiara `pinSync`, il Rust serializza `pin_sync`, e il popup mostra una
// corsia perennemente vuota invece di un errore. Il caso è concreto perché lo
// snapshot viaggia sia come risposta di `get_hubcap_monitor_status` sia come
// payload dell'evento push `sync://status-changed`.

use std::collections::BTreeSet;

use crate::core::hubcap_update_monitor::{LaneStatus, MonitorStatusSnapshot, PendingTaskInfo};

/// Chiavi di primo livello di un oggetto JSON.
fn json_object_keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .expect("il valore serializzato deve essere un oggetto")
        .keys()
        .cloned()
        .collect()
}

/// Chiavi dichiarate da `export interface <name> { … }` in un file TS.
///
/// Lettura volutamente semplice: una chiave per riga, nella forma
/// `nome?: tipo;` — è così che sono scritte le interfacce mirror di questo
/// progetto. Se il formato cambia, il test lo dice invece di passare di
/// nascosto (nessuna chiave trovata = fallimento).
fn ts_interface_keys(source: &str, name: &str) -> BTreeSet<String> {
    let header = format!("export interface {name} {{");
    let start = source
        .find(&header)
        .unwrap_or_else(|| panic!("interfaccia `{name}` non trovata nel file TS"));
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
        .expect("AetherDesk/ sopra src-tauri/")
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
        "\nChiavi dello snapshot divergenti tra Rust e TypeScript.\n  solo Rust: {:?}\n  solo TS:   {:?}\n\
         Allineare src/types/sync.ts oppure #[serde(rename_all)] in core/hubcap_update_monitor.rs.\n",
        rust_top.difference(&ts_top).collect::<Vec<_>>(),
        ts_top.difference(&rust_top).collect::<Vec<_>>(),
    );

    let lane = json
        .get("pinSync")
        .expect("la corsia pinSync deve essere serializzata in camelCase");
    let rust_lane = json_object_keys(lane);
    let ts_lane = ts_interface_keys(&source, "LaneStatus");
    assert_eq!(
        rust_lane, ts_lane,
        "\nChiavi di LaneStatus divergenti.\n  solo Rust: {:?}\n  solo TS:   {:?}\n",
        rust_lane.difference(&ts_lane).collect::<Vec<_>>(),
        ts_lane.difference(&rust_lane).collect::<Vec<_>>(),
    );

    let task = lane
        .get("pending")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .expect("pending deve contenere almeno un task nel campione");
    let rust_task = json_object_keys(task);
    let ts_task = ts_interface_keys(&source, "PendingTaskInfo");
    assert_eq!(
        rust_task, ts_task,
        "\nChiavi di PendingTaskInfo divergenti.\n  solo Rust: {:?}\n  solo TS:   {:?}\n",
        rust_task.difference(&ts_task).collect::<Vec<_>>(),
        ts_task.difference(&rust_task).collect::<Vec<_>>(),
    );
}

#[test]
fn monitor_status_snapshot_serializes_every_field_even_when_empty() {
    // Il popup legge le quattro corsie senza controlli di presenza: se un campo
    // `Option` venisse omesso quando è `None` (skip_serializing_if), la UI
    // riceverebbe `undefined` dove si aspetta un numero o un array.
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
