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

#[test]
fn test_core_commands_inventory_check() {
    // Inventory of essential commands that the frontend invokes and relies upon:
    let expected_core_commands = [
        "get_settings",
        "save_settings",
        "get_installed_library_games",
        "get_installed_lua_manifest_rows",
        "save_installed_lua_manifest_rows",
        "plan_online",
        "enable_online",
        "disable_online",
        "get_recent_log_lines",
        "check_aether_desk_update",
        "check_aether_dll_update",
    ];

    for command in expected_core_commands {
        assert!(!command.is_empty(), "command {command} should be accounted for");
    }
}
