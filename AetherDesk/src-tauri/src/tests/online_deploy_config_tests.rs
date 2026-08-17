//! Test di `online::config` (golden test dell'ini) e del deploy/revert
//! roundtrip su gioco fittizio.
//!
//! NOTA: file condiviso tra la repo e l'harness standalone — non cambiare
//! i path di import senza aggiornare entrambi.

use crate::online::bundle::Uco2Bundle;
use crate::online::config::{build_ini, harvest_dlc, COHERENCE_SHARED_KEY};
use crate::online::deploy::{backup_dir_for, deploy, Journal};
use crate::online::detect::GameInspector;
use crate::online::revert::disable;
use crate::online::state::OnlineStateStore;
use crate::online::types::{
    CoherenceOptions, EosOptions, OnlineEnableRequest, PhotonOptions, PlayfabOptions,
};
use std::fs;
use std::path::Path;

fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// Mini-gioco Unity Mono con Photon Realtime + Voice + PlayFab.
fn unity_photon_voice_playfab_fixture(root: &Path) {
    write(root, "MyGame.exe", b"MZfake");
    write(root, "MyGame_Data/Managed/Assembly-CSharp.dll", b"asm");
    write(root, "MyGame_Data/Plugins/x86_64/steam_api64.dll", b"original-dll");
    write(root, "MyGame_Data/Managed/PhotonRealtime.dll", b"photon");
    write(root, "MyGame_Data/Managed/PhotonVoice.dll", b"voice");
    write(root, "MyGame_Data/Managed/PlayFabAllSDK.dll", b"playfab");
    // DLC harvest sorgente 1: configs.app.ini stile Goldberg.
    write(root, "configs.app.ini", b"[app::dlcs]\n211=Half-Life 2: Deathmatch\n212=Half-Life 2: Lost Coast\n");
}

/// Bundle UCO2 fittizio (layout release).
fn bundle_fixture(dir: &Path) -> Uco2Bundle {
    write(dir, "x64/steam_api64.dll", b"uco2-x64");
    write(dir, "x86/steam_api.dll", b"uco2-x86");
    write(dir, "plugins/photon_universal.dll", b"photon-plugin");
    write(dir, "plugins/playfab_universal.dll", b"playfab-plugin");
    write(dir, "VERSION", b"v9.9.9\n");
    Uco2Bundle::open(dir.to_path_buf()).expect("bundle fixture valid")
}

fn base_request() -> OnlineEnableRequest {
    OnlineEnableRequest {
        og_app_id: 1144200,
        spoof_app_id: 480,
        photon: PhotonOptions {
            realtime_guid: "rt-guid".to_string(),
            voice_guid: "vo-guid".to_string(),
            fusion_guid: String::new(),
        },
        eos: EosOptions::default(),
        playfab: PlayfabOptions {
            title_id: "TITLEID".to_string(),
        },
        coherence: CoherenceOptions::default(),
        deploy_eos_custom: false,
    }
}

// ---------------------------------------------------------------------------
// Golden test dell'ini
// ---------------------------------------------------------------------------

#[test]
fn ini_golden_unity_photon_voice_playfab() {
    let tmp = tempfile::tempdir().unwrap();
    unity_photon_voice_playfab_fixture(tmp.path());

    let detection = GameInspector::inspect(tmp.path()).unwrap();
    let request = base_request();
    let dlc = harvest_dlc(&detection.game_root, &detection.ini_dir);

    // Harvest: deve trovare configs.app.ini con 2 entry.
    assert_eq!(
        dlc,
        vec![
            ("211".to_string(), "Half-Life 2: Deathmatch".to_string()),
            ("212".to_string(), "Half-Life 2: Lost Coast".to_string()),
        ]
    );

    let ini = build_ini(&detection, &request, &dlc);
    let expected = "[Settings]\r\n\
AppId=480\r\n\
ogAppId=1144200\r\n\
PluginsFolder=plugins\r\n\
\r\n\
[DLC]\r\n\
; UnlockAll answers any \"do I own this DLC?\" check, for any id, so DLC\r\n\
; works without knowing what the ids are.\r\n\
; The \"appid=name\" lines below are what a game reads when it ENUMERATES\r\n\
; its DLC to build a menu. Both work together - UnlockAll is the fallback.\r\n\
UnlockAll=true\r\n\
211=Half-Life 2: Deathmatch\r\n\
212=Half-Life 2: Lost Coast\r\n\
\r\n\
[Realtime]\r\n\
PhotonAppIdRealtime=rt-guid\r\n\
PhotonAppIdVoice=vo-guid\r\n\
ForcedAuthType=0\r\n\
\r\n\
[PlayFab]\r\n\
TitleId=TITLEID\r\n";
    assert_eq!(ini, expected);
}

#[test]
fn ini_fusion_and_stub_and_coherence_shared() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "FGame.exe", b"MZ");
    write(tmp.path(), "FGame_Data/Managed/Fusion.Realtime.dll", b"fusion");
    write(tmp.path(), "FGame_Data/StreamingAssets/combined.schema", b"{}");

    let detection = GameInspector::inspect(tmp.path()).unwrap();
    let mut request = base_request();
    request.photon.fusion_guid = "fu-guid".to_string();
    request.coherence.use_shared = true;

    let ini = build_ini(&detection, &request, &[]);
    assert!(ini.contains("[Fusion]\r\nPhotonAppIdFusion=fu-guid\r\nForcedAuthType=0"));
    let coherence_section = format!(
        "[Coherence]\r\nForceGuestLogin=true\r\nRuntimeKey={}\r\nLocalMode=false",
        COHERENCE_SHARED_KEY
    );
    assert!(ini.contains(&coherence_section));
    // Niente sezioni non rilevate.
    assert!(!ini.contains("[Realtime]"));
    assert!(!ini.contains("[EOS]"));
    assert!(!ini.contains("[PlayFab]"));
}

#[test]
fn ini_steam_only_has_no_backend_sections() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "SGame.exe", b"MZ");
    write(tmp.path(), "SGame_Data/Managed/Assembly-CSharp.dll", b"asm");

    let detection = GameInspector::inspect(tmp.path()).unwrap();
    let ini = build_ini(&detection, &base_request(), &[]);
    assert!(!ini.contains("PhotonAppId"));
    assert!(!ini.contains("[EOS]"));
    assert!(!ini.contains("[Coherence]"));
    assert!(!ini.contains("[PlayFab]"));
}

#[test]
fn dlc_harvest_falls_back_to_numeric_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "G.exe", b"MZ");
    write(tmp.path(), "G_Data/Managed/Assembly-CSharp.dll", b"asm");
    // Nessun configs.app.ini; cartelle numeriche nella dir di gioco.
    write(tmp.path(), "211/readme.txt", b"dlc");
    write(tmp.path(), "212/readme.txt", b"dlc");
    write(tmp.path(), "not-a-dlc/readme.txt", b"x");

    let detection = GameInspector::inspect(tmp.path()).unwrap();
    let dlc = harvest_dlc(&detection.game_root, &detection.ini_dir);
    assert_eq!(
        dlc,
        vec![
            ("211".to_string(), "DLC 211".to_string()),
            ("212".to_string(), "DLC 212".to_string()),
        ]
    );
}

// ---------------------------------------------------------------------------
// Deploy / revert roundtrip
// ---------------------------------------------------------------------------

#[test]
fn deploy_then_disable_restores_everything() {
    let game = tempfile::tempdir().unwrap();
    unity_photon_voice_playfab_fixture(game.path());
    let bundle_dir = tempfile::tempdir().unwrap();
    let bundle = bundle_fixture(bundle_dir.path());
    let data = tempfile::tempdir().unwrap();
    let backup_root = data.path().join("backup");
    let state_path = data.path().join("state/uc_online2.json");

    let detection = GameInspector::inspect(game.path()).unwrap();
    let steam_api_dir = detection.steam_api_dir.clone().unwrap();
    let ini_path = detection.ini_dir.join("union-crax.ini");
    let original_dll = steam_api_dir.join("steam_api64.dll");

    // --- enable ---
    let record = deploy(
        1144200,
        &detection,
        &bundle,
        &base_request(),
        &backup_root,
        &state_path,
    )
    .expect("deploy must succeed");

    assert_eq!(record.app_id, 1144200);
    assert_eq!(record.og_app_id, 1144200);
    assert_eq!(record.arch, crate::online::types::GameArch::X64);
    assert_eq!(
        record.backends_deployed,
        vec!["photon_universal", "playfab_universal"]
    );
    assert_eq!(record.bundle_version.as_deref(), Some("v9.9.9"));

    // DLL sostituita con quella UCO2.
    assert_eq!(fs::read(&original_dll).unwrap(), b"uco2-x64");
    // L'originale è in backup.
    let backup_dir = backup_dir_for(&backup_root, 1144200);
    assert!(backup_dir.join("original/steam_api").is_file());
    // Ini scritto.
    let ini_content = fs::read_to_string(&ini_path).unwrap();
    assert!(ini_content.contains("ogAppId=1144200"));
    assert!(ini_content.contains("PhotonAppIdRealtime=rt-guid"));
    // Plugin deployati.
    assert!(detection.ini_dir.join("plugins/photon_universal.dll").is_file());
    assert!(detection.ini_dir.join("plugins/playfab_universal.dll").is_file());
    // Journal presente.
    assert!(!Journal::load(&backup_dir).entries.is_empty());
    // Stato persistito.
    let store = OnlineStateStore::load(&state_path);
    assert!(store.get(1144200).is_some());

    // --- idempotenza: secondo enable non fa doppio backup ---
    let record2 = deploy(
        1144200,
        &detection,
        &bundle,
        &base_request(),
        &backup_root,
        &state_path,
    )
    .expect("second deploy must succeed (config refresh)");
    assert_eq!(record2.backends_deployed, record.backends_deployed);
    assert!(!backup_dir.join("original/steam_api.1").exists(), "no double backup");

    // --- disable: ripristino completo ---
    disable(1144200, &backup_root, &state_path).expect("disable must succeed");

    assert_eq!(fs::read(&original_dll).unwrap(), b"original-dll");
    assert!(!ini_path.exists(), "ini removed");
    assert!(!detection.ini_dir.join("plugins").exists(), "plugins dir removed");
    let store = OnlineStateStore::load(&state_path);
    assert!(store.get(1144200).is_none(), "record removed");
    assert!(!backup_dir.exists(), "backup dir removed");
}

#[test]
fn deploy_neutralizes_conflicts_and_disable_restores_them() {
    let game = tempfile::tempdir().unwrap();
    unity_photon_voice_playfab_fixture(game.path());
    // Conflitto: SteamFix64.dll accanto alla DLL Steamworks.
    let steam_api_dir = game.path().join("MyGame_Data/Plugins/x86_64");
    write(&steam_api_dir, "SteamFix64.dll", b"fix");
    let bundle_dir = tempfile::tempdir().unwrap();
    let bundle = bundle_fixture(bundle_dir.path());
    let data = tempfile::tempdir().unwrap();
    let backup_root = data.path().join("backup");
    let state_path = data.path().join("state/uc_online2.json");

    let detection = GameInspector::inspect(game.path()).unwrap();
    assert!(detection
        .conflicts
        .iter()
        .any(|c| matches!(c, crate::online::types::Conflict::SteamFix(_))));

    deploy(1144200, &detection, &bundle, &base_request(), &backup_root, &state_path)
        .expect("deploy");

    // Neutralizzato in modo reversibile.
    assert!(!steam_api_dir.join("SteamFix64.dll").exists());
    assert!(steam_api_dir.join("SteamFix64.dll.uco-disabled").is_file());

    disable(1144200, &backup_root, &state_path).expect("disable");

    // Ripristinato.
    assert!(steam_api_dir.join("SteamFix64.dll").is_file());
    assert!(!steam_api_dir.join("SteamFix64.dll.uco-disabled").exists());
}
