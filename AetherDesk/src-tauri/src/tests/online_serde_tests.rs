//! Test di serializzazione (anti-regressione per il bug snake_case/camelCase).
//!
//! Tutte le struct che attraversano il confine Tauri (Rust ⇄ UI) DEVONO
//! serializzare in camelCase e deserializzare da camelCase. Questo file
//! blocca la classe di bug che rendeva "Enable Online" permanentemente
//! disabilitato (la UI leggeva `bundleOk`, il JSON aveva `bundle_ok`).
//!
//! NOTA: file condiviso tra la repo e l'harness standalone.

use crate::online::types::{
    BackendReport, CoherenceOptions, DetectionReport, Engine, EosOptions, GameArch,
    OnlineEnableRequest, OnlinePlan, OnlineRecord, OnlineStatus, OnlineStateKind, PhotonFlavor,
    PhotonOptions, PlayfabOptions, Prerequisites,
};
use std::path::PathBuf;

fn detection_sample() -> DetectionReport {
    DetectionReport {
        game_root: PathBuf::from("C:\\Games\\MyGame"),
        engine: Engine::Unity,
        arch: GameArch::X64,
        game_exe: Some(PathBuf::from("C:\\Games\\MyGame\\MyGame.exe")),
        unity_data_dir: Some(PathBuf::from("C:\\Games\\MyGame\\MyGame_Data")),
        steam_api_dir: Some(PathBuf::from("C:\\Games\\MyGame\\MyGame_Data\\Plugins\\x86_64")),
        ini_dir: PathBuf::from("C:\\Games\\MyGame"),
        backends: BackendReport {
            photon: PhotonFlavor::Realtime,
            photon_voice: true,
            eos: false,
            playfab: true,
            coherence: false,
        },
        conflicts: Vec::new(),
        steamless_applied: false,
        steamstub_detected: false,
        warnings: vec!["w".to_string()],
    }
}

#[test]
fn prerequisites_serializes_camel_case() {
    let pre = Prerequisites {
        bundle_ok: true,
        bundle_version: Some("v1.19.3".to_string()),
        steam_api_dir_writable: true,
        errors: vec![],
    };
    let json = serde_json::to_value(&pre).unwrap();
    assert!(json.get("bundleOk").is_some(), "bundleOk manca: {json}");
    assert!(json.get("bundleVersion").is_some());
    assert!(json.get("steamApiDirWritable").is_some());
    assert!(json.get("errors").is_some());
    assert!(json.get("bundle_ok").is_none(), "snake_case non deve esistere: {json}");
}

#[test]
fn detection_report_serializes_camel_case() {
    let json = serde_json::to_value(detection_sample()).unwrap();
    for key in ["gameRoot", "engine", "arch", "gameExe", "unityDataDir", "steamApiDir", "iniDir", "steamlessApplied", "steamstubDetected", "warnings"] {
        assert!(json.get(key).is_some(), "chiave '{key}' manca: {json}");
    }
    let backends = json.get("backends").unwrap();
    assert!(backends.get("photonVoice").is_some(), "photonVoice manca: {backends}");
    assert!(backends.get("photon_voice").is_none());
}

#[test]
fn online_plan_serializes_camel_case() {
    let plan = OnlinePlan {
        detection: detection_sample(),
        prerequisites: Prerequisites::default(),
        current: None,
        notices: vec!["n".to_string()],
    };
    let json = serde_json::to_value(&plan).unwrap();
    for key in ["detection", "prerequisites", "current", "notices"] {
        assert!(json.get(key).is_some(), "chiave '{key}' manca: {json}");
    }
    // Il bug storico: bundleOk deve essere raggiungibile via prerequisites.bundleOk.
    assert!(json["prerequisites"].get("bundleOk").is_some());
}

#[test]
fn enable_request_deserializes_from_camel_case() {
    // Questo è ESATTAMENTE il JSON che la UI (OnlinePanel.tsx) invia.
    let json = serde_json::json!({
        "ogAppId": 1144200,
        "spoofAppId": 480,
        "verboseLog": true,
        "emulateTicket": false,
        "warnOverlayDisabled": false,
        "sdr": false,
        "unlockAllDlc": true,
        "deployPhoton": false,
        "photon": { "realtimeGuid": "rt", "voiceGuid": "vo", "fusionGuid": "fu" },
        "eos": { "productId": "p", "sandboxId": "s", "deploymentId": "d", "clientId": "c", "clientSecret": "sec" },
        "playfab": { "titleId": "TITLE" },
        "coherence": { "runtimeKey": "key", "useShared": true },
        "deployEosCustom": true
    });
    let request: OnlineEnableRequest = serde_json::from_value(json).expect("deve deserializzare");
    assert!(request.load_overlay, "LoadOverlay default true se assente");
    assert!(!request.log_overlay);
    assert!(!request.get_stubbed_lol);
    assert!(request.client.is_empty());
    assert!(request.deploy_overlay_proxy);
    assert!(!request.playfab.use_shared);
    assert_eq!(request.og_app_id, 1144200);
    assert_eq!(request.spoof_app_id, 480);
    assert!(request.verbose_log);
    assert!(!request.emulate_ticket);
    assert!(!request.warn_overlay_disabled);
    assert!(!request.sdr);
    assert!(request.unlock_all_dlc);
    assert!(!request.deploy_photon);
    assert_eq!(request.photon.realtime_guid, "rt");
    assert_eq!(request.photon.voice_guid, "vo");
    assert_eq!(request.photon.fusion_guid, "fu");
    assert_eq!(request.eos.product_id, "p");
    assert_eq!(request.eos.client_secret, "sec");
    assert_eq!(request.playfab.title_id, "TITLE");
    assert_eq!(request.coherence.runtime_key, "key");
    assert!(request.coherence.use_shared);
    assert!(request.deploy_eos_custom);
}

#[test]
fn options_deserialize_from_camel_case() {
    let photon: PhotonOptions =
        serde_json::from_value(serde_json::json!({"realtimeGuid": "a", "voiceGuid": "b", "fusionGuid": "c"}))
            .unwrap();
    assert_eq!(photon.realtime_guid, "a");
    assert_eq!(photon.fusion_guid, "c");

    let eos: EosOptions = serde_json::from_value(serde_json::json!({
        "productId": "p", "sandboxId": "s", "deploymentId": "d",
        "clientId": "c", "clientSecret": "sec"
    }))
    .unwrap();
    assert_eq!(eos.product_id, "p");
    assert_eq!(eos.deployment_id, "d");
    assert_eq!(eos.client_secret, "sec");

    let pf: PlayfabOptions = serde_json::from_value(serde_json::json!({"titleId": "T"})).unwrap();
    assert_eq!(pf.title_id, "T");

    let coh: CoherenceOptions =
        serde_json::from_value(serde_json::json!({"runtimeKey": "k", "useShared": true})).unwrap();
    assert_eq!(coh.runtime_key, "k");
    assert!(coh.use_shared);
}

#[test]
fn record_and_status_serialize_camel_case() {
    let record = OnlineRecord {
        app_id: 42,
        enabled_at: 1,
        bundle_version: Some("v1.19.3".to_string()),
        og_app_id: 440,
        spoof_app_id: 480,
        ini_path: PathBuf::from("C:\\ini"),
        steam_api_path: PathBuf::from("C:\\dll"),
        arch: GameArch::X86,
        backends_deployed: vec!["photon_universal".to_string()],
        backup_dir: PathBuf::from("C:\\backup"),
        overlay_proxy_path: None,
    };
    let json = serde_json::to_value(&record).unwrap();
    for key in ["appId", "enabledAt", "bundleVersion", "ogAppId", "spoofAppId", "iniPath", "steamApiPath", "arch", "backendsDeployed", "backupDir", "overlayProxyPath"] {
        assert!(json.get(key).is_some(), "chiave '{key}' manca: {json}");
    }

    let status = OnlineStatus {
        state: OnlineStateKind::NotConfigured,
        record: None,
    };
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["state"], "not_configured");
}
