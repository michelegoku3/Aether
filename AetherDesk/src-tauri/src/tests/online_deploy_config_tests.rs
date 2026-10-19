//! Tests for `online::config` (ini golden test) and the deploy/revert
//! roundtrip on a fake game.
//!
//! NOTE: file shared between the repo and the standalone harness — do not
//! change import paths without updating both.

use crate::online::bundle::Uco2Bundle;
use crate::online::config::{build_ini, harvest_dlc, COHERENCE_SHARED_KEY};
use crate::online::deploy::{backup_dir_for, deploy, Journal};
use crate::online::detect::GameInspector;
use crate::online::revert::disable;
use crate::online::state::OnlineStateStore;
use crate::online::types::{OnlineEnableRequest, PhotonOptions, PlayfabOptions};
use std::fs;
use std::path::Path;

fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// Mini Mono Unity game with Photon Realtime + Voice + PlayFab.
fn unity_photon_voice_playfab_fixture(root: &Path) {
    write(root, "MyGame.exe", b"MZfake");
    write(root, "MyGame_Data/Managed/Assembly-CSharp.dll", b"asm");
    write(
        root,
        "MyGame_Data/Plugins/x86_64/steam_api64.dll",
        b"original-dll",
    );
    write(root, "MyGame_Data/Managed/PhotonRealtime.dll", b"photon");
    write(root, "MyGame_Data/Managed/PhotonVoice.dll", b"voice");
    write(root, "MyGame_Data/Managed/PlayFabAllSDK.dll", b"playfab");
    // DLC harvest sorgente 1: configs.app.ini stile Goldberg.
    write(
        root,
        "configs.app.ini",
        b"[app::dlcs]\n211=Half-Life 2: Deathmatch\n212=Half-Life 2: Lost Coast\n",
    );
}

/// Fake UCO2 bundle (release layout).
fn bundle_fixture(dir: &Path) -> Uco2Bundle {
    write(dir, "x64/steam_api64.dll", b"uco2-x64");
    write(dir, "x86/steam_api.dll", b"uco2-x86");
    write(dir, "plugins/photon_universal.dll", b"photon-plugin");
    write(dir, "plugins/playfab_universal.dll", b"playfab-plugin");
    write(dir, "plugins/overlay_proxy.dll", b"overlay-proxy");
    write(dir, "VERSION", b"v9.9.9\n");
    Uco2Bundle::open(dir.to_path_buf()).expect("bundle fixture valid")
}

fn base_request() -> OnlineEnableRequest {
    OnlineEnableRequest {
        og_app_id: 1144200,
        deploy_photon: true,
        photon: PhotonOptions {
            realtime_guid: "rt-guid".to_string(),
            voice_guid: "vo-guid".to_string(),
            fusion_guid: String::new(),
        },
        playfab: PlayfabOptions {
            title_id: "TITLEID".to_string(),
            use_shared: false,
        },
        ..OnlineEnableRequest::default()
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

    // Harvest: must find configs.app.ini with 2 entries.
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
VerboseLog=true\r\n\
EmulateTicket=false\r\n\
WarnOverlayDisabled=false\r\n\
SDR=false\r\n\
LoadOverlay=true\r\n\
LogOverlay=false\r\n\
GetStubbedLol=false\r\n\
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
    write(
        tmp.path(),
        "FGame_Data/Managed/Fusion.Realtime.dll",
        b"fusion",
    );
    write(
        tmp.path(),
        "FGame_Data/StreamingAssets/combined.schema",
        b"{}",
    );

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
    // No undetected sections.
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
fn ini_settings_toggles_are_written() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "TGame.exe", b"MZ");
    write(tmp.path(), "TGame_Data/Managed/Assembly-CSharp.dll", b"asm");

    let detection = GameInspector::inspect(tmp.path()).unwrap();
    let mut request = base_request();
    request.verbose_log = false;
    request.emulate_ticket = true;
    request.warn_overlay_disabled = true;
    request.sdr = true;
    request.load_overlay = false;
    request.log_overlay = true;
    request.get_stubbed_lol = true;
    request.client = "017".to_string();
    request.unlock_all_dlc = false;

    let ini = build_ini(&detection, &request, &[]);
    assert!(ini.contains("VerboseLog=false\r\n"));
    assert!(ini.contains("EmulateTicket=true\r\n"));
    assert!(ini.contains("WarnOverlayDisabled=true\r\n"));
    assert!(ini.contains("SDR=true\r\n"));
    assert!(ini.contains("LoadOverlay=false\r\n"));
    assert!(ini.contains("LogOverlay=true\r\n"));
    assert!(ini.contains("GetStubbedLol=true\r\n"));
    assert!(ini.contains("Client=017\r\n"));
    assert!(ini.contains("UnlockAll=false\r\n"));
}

#[test]
fn ini_omits_empty_client() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "TGame.exe", b"MZ");
    write(tmp.path(), "TGame_Data/Managed/Assembly-CSharp.dll", b"asm");
    let detection = GameInspector::inspect(tmp.path()).unwrap();
    let ini = build_ini(&detection, &base_request(), &[]);
    assert!(!ini.contains("Client="));
}

#[test]
fn ini_playfab_shared_title() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "PGame.exe", b"MZ");
    write(tmp.path(), "PGame_Data/Managed/PlayFabAllSDK.dll", b"pf");
    write(tmp.path(), "PGame_Data/Managed/Assembly-CSharp.dll", b"asm");
    let detection = GameInspector::inspect(tmp.path()).unwrap();
    let mut request = base_request();
    request.playfab.use_shared = true;
    let ini = build_ini(&detection, &request, &[]);
    assert!(ini.contains("TitleId=1D861F"));
}

#[test]
fn ini_photon_section_requires_deploy_photon() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "PGame.exe", b"MZ");
    write(
        tmp.path(),
        "PGame_Data/Managed/PhotonRealtime.dll",
        b"photon",
    );
    write(tmp.path(), "PGame_Data/Managed/Assembly-CSharp.dll", b"asm");

    let detection = GameInspector::inspect(tmp.path()).unwrap();
    let mut request = base_request();
    request.deploy_photon = false;

    let ini = build_ini(&detection, &request, &[]);
    assert!(!ini.contains("[Realtime]"));
    assert!(!ini.contains("PhotonAppIdRealtime"));

    request.deploy_photon = true;
    let ini = build_ini(&detection, &request, &[]);
    assert!(ini.contains("[Realtime]\r\nPhotonAppIdRealtime=rt-guid"));
}

#[test]
fn dlc_harvest_falls_back_to_numeric_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "G.exe", b"MZ");
    write(tmp.path(), "G_Data/Managed/Assembly-CSharp.dll", b"asm");
    // No configs.app.ini; numbered folders in the game dir.
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

    // DLL replaced with the UCO2 one.
    assert_eq!(fs::read(&original_dll).unwrap(), b"uco2-x64");
    // The original is in backup.
    let backup_dir = backup_dir_for(&backup_root, 1144200);
    assert!(backup_dir.join("original/steam_api").is_file());
    // Ini scritto.
    let ini_content = fs::read_to_string(&ini_path).unwrap();
    assert!(ini_content.contains("ogAppId=1144200"));
    assert!(ini_content.contains("PhotonAppIdRealtime=rt-guid"));
    // Plugin deployati.
    assert!(detection
        .ini_dir
        .join("plugins/photon_universal.dll")
        .is_file());
    assert!(detection
        .ini_dir
        .join("plugins/playfab_universal.dll")
        .is_file());
    // Overlay proxy Unity x64.
    assert!(detection.ini_dir.join("version.dll").is_file());
    assert_eq!(
        fs::read(detection.ini_dir.join("version.dll")).unwrap(),
        b"overlay-proxy"
    );
    // Journal present.
    assert!(!Journal::load(&backup_dir).entries.is_empty());
    // Persisted state.
    let store = OnlineStateStore::load(&state_path);
    assert!(store.get(1144200).is_some());

    // --- idempotency: second enable performs no double backup ---
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
    assert!(
        !backup_dir.join("original/steam_api.1").exists(),
        "no double backup"
    );

    // --- disable: ripristino completo ---
    disable(1144200, &backup_root, &state_path, None, false).expect("disable must succeed");

    assert_eq!(fs::read(&original_dll).unwrap(), b"original-dll");
    assert!(!ini_path.exists(), "ini removed");
    assert!(
        !detection.ini_dir.join("plugins").exists(),
        "plugins dir removed"
    );
    let store = OnlineStateStore::load(&state_path);
    assert!(store.get(1144200).is_none(), "record removed");
    assert!(!backup_dir.exists(), "backup dir removed");
}

#[test]
fn deploy_neutralizes_conflicts_and_disable_restores_them() {
    let game = tempfile::tempdir().unwrap();
    unity_photon_voice_playfab_fixture(game.path());
    // Conflict: SteamFix64.dll next to the Steamworks DLL.
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

    deploy(
        1144200,
        &detection,
        &bundle,
        &base_request(),
        &backup_root,
        &state_path,
    )
    .expect("deploy");

    // Quarantined in the backup (no *.uco-disabled in the game tree).
    assert!(!steam_api_dir.join("SteamFix64.dll").exists());
    assert!(!steam_api_dir.join("SteamFix64.dll.uco-disabled").exists());
    let backup_dir = backup_dir_for(&backup_root, 1144200);
    assert!(backup_dir
        .join("original/conflict_SteamFix64.dll")
        .is_file());

    disable(1144200, &backup_root, &state_path, None, false).expect("disable must succeed");

    // Ripristinato.
    assert!(steam_api_dir.join("SteamFix64.dll").is_file());
    assert!(!steam_api_dir.join("SteamFix64.dll.uco-disabled").exists());
}

// Regression fixtures exercise the real deploy/engine/filesystem, never live games.
fn deployed_with_original_plugins() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    crate::online::types::OnlineRecord,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let game = tempfile::tempdir().unwrap();
    unity_photon_voice_playfab_fixture(game.path());
    write(game.path(), "plugins/original.dll", b"original plugin");
    // Even a marker-shaped original must not be swept after restoration.
    write(
        game.path(),
        "plugins/EOS_custom.dll",
        b"original marker-shaped plugin",
    );
    let bundle_dir = tempfile::tempdir().unwrap();
    let bundle = bundle_fixture(bundle_dir.path());
    let data = tempfile::tempdir().unwrap();
    let backup_root = data.path().join("backup");
    let state_path = data.path().join("state/uc_online2.json");
    let detection = GameInspector::inspect(game.path()).unwrap();
    let record = deploy(
        1144200,
        &detection,
        &bundle,
        &base_request(),
        &backup_root,
        &state_path,
    )
    .unwrap();
    (game, data, record, backup_root, state_path)
}

#[test]
fn disable_restores_preexisting_plugins_and_does_not_sweep_originals() {
    let (game, _data, record, backup_root, state_path) = deployed_with_original_plugins();
    let result = crate::online::engine::OnlineEngine::disable(
        1144200,
        &backup_root,
        &state_path,
        Some(game.path()),
        false,
    )
    .unwrap();
    assert!(result.success);
    assert_eq!(
        fs::read(game.path().join("plugins/original.dll")).unwrap(),
        b"original plugin"
    );
    assert_eq!(
        fs::read(game.path().join("plugins/EOS_custom.dll")).unwrap(),
        b"original marker-shaped plugin"
    );
    assert_eq!(fs::read(&record.steam_api_path).unwrap(), b"original-dll");
    assert!(!record.backup_dir.exists());
    assert!(OnlineStateStore::load(&state_path).get(1144200).is_none());
    assert!(
        crate::online::engine::OnlineEngine::disable(
            1144200,
            &backup_root,
            &state_path,
            None,
            false
        )
        .unwrap()
        .success
    );
}

#[test]
fn disable_collision_keeps_backup_and_state_then_retries_successfully() {
    let (game, _data, record, backup_root, state_path) = deployed_with_original_plugins();
    let collision = game.path().join("plugins/user-sentinel.txt");
    fs::write(&collision, b"user sentinel").unwrap();
    assert!(crate::online::engine::OnlineEngine::disable(
        1144200,
        &backup_root,
        &state_path,
        Some(game.path()),
        false
    )
    .is_err());
    assert_eq!(fs::read(&collision).unwrap(), b"user sentinel");
    assert_eq!(
        fs::read(record.backup_dir.join("original/plugins/original.dll")).unwrap(),
        b"original plugin"
    );
    assert!(!Journal::load(&record.backup_dir).entries.is_empty());
    assert!(OnlineStateStore::load(&state_path).get(1144200).is_some());
    let kept = game.path().join("kept-sentinel.txt");
    fs::rename(collision, &kept).unwrap();
    assert!(
        crate::online::engine::OnlineEngine::disable(
            1144200,
            &backup_root,
            &state_path,
            Some(game.path()),
            false
        )
        .unwrap()
        .success
    );
    assert_eq!(fs::read(kept).unwrap(), b"user sentinel");
    assert_eq!(
        fs::read(game.path().join("plugins/original.dll")).unwrap(),
        b"original plugin"
    );
}

#[test]
fn failed_deployed_removal_is_reported_and_does_not_discard_recovery() {
    let (game, _data, record, backup_root, state_path) = deployed_with_original_plugins();
    let blocked = game.path().join("plugins/photon_universal.dll");
    fs::remove_file(&blocked).unwrap();
    fs::create_dir(&blocked).unwrap();
    fs::write(blocked.join("sentinel.txt"), b"do not delete").unwrap();
    assert!(crate::online::engine::OnlineEngine::disable(
        1144200,
        &backup_root,
        &state_path,
        Some(game.path()),
        false
    )
    .is_err());
    assert_eq!(
        fs::read(blocked.join("sentinel.txt")).unwrap(),
        b"do not delete"
    );
    assert!(!Journal::load(&record.backup_dir).entries.is_empty());
    assert!(OnlineStateStore::load(&state_path).get(1144200).is_some());
    fs::rename(&blocked, game.path().join("kept-blocker")).unwrap();
    assert!(
        crate::online::engine::OnlineEngine::disable(
            1144200,
            &backup_root,
            &state_path,
            Some(game.path()),
            false
        )
        .unwrap()
        .success
    );
    assert_eq!(fs::read(&record.steam_api_path).unwrap(), b"original-dll");
    assert_eq!(
        fs::read(game.path().join("kept-blocker/sentinel.txt")).unwrap(),
        b"do not delete"
    );
}

#[test]
fn state_save_failure_keeps_empty_journal_for_a_safe_retry() {
    let (game, _data, record, backup_root, state_path) = deployed_with_original_plugins();
    let blocked_temp = std::path::PathBuf::from(format!("{}.tmp", state_path.display()));
    fs::create_dir(&blocked_temp).unwrap();
    assert!(crate::online::engine::OnlineEngine::disable(
        1144200,
        &backup_root,
        &state_path,
        Some(game.path()),
        false
    )
    .is_err());
    assert!(record.backup_dir.join("journal.json").is_file());
    assert!(Journal::load(&record.backup_dir).entries.is_empty());
    assert!(OnlineStateStore::load(&state_path).get(1144200).is_some());
    assert_eq!(fs::read(&record.steam_api_path).unwrap(), b"original-dll");
    fs::remove_dir(blocked_temp).unwrap();
    assert!(
        crate::online::engine::OnlineEngine::disable(
            1144200,
            &backup_root,
            &state_path,
            Some(game.path()),
            false
        )
        .unwrap()
        .success
    );
    assert_eq!(fs::read(&record.steam_api_path).unwrap(), b"original-dll");
    assert!(!record.backup_dir.exists());
}

#[test]
fn corrupt_journal_still_refuses_heuristic_deletion() {
    let (game, _data, record, backup_root, state_path) = deployed_with_original_plugins();
    fs::write(record.backup_dir.join("journal.json"), b"not valid JSON").unwrap();
    let state_before = fs::read(&state_path).unwrap();

    assert!(crate::online::engine::OnlineEngine::disable(
        1144200,
        &backup_root,
        &state_path,
        Some(game.path()),
        false
    )
    .is_err());

    assert_eq!(fs::read(&record.steam_api_path).unwrap(), b"uco2-x64");
    assert_eq!(
        fs::read(record.backup_dir.join("original/steam_api")).unwrap(),
        b"original-dll"
    );
    assert_eq!(fs::read(&state_path).unwrap(), state_before);
}

#[test]
fn missing_journal_needs_confirmation_then_removes_uco2_and_keeps_originals() {
    let (game, _data, record, backup_root, state_path) = deployed_with_original_plugins();
    fs::remove_file(record.backup_dir.join("journal.json")).unwrap();
    let state_before = fs::read(&state_path).unwrap();

    // First call: nothing is deleted, the UI must confirm.
    let first = crate::online::engine::OnlineEngine::disable(
        1144200,
        &backup_root,
        &state_path,
        Some(game.path()),
        false,
    )
    .unwrap();
    assert!(!first.success);
    assert!(first.confirmation_required);
    assert_eq!(fs::read(&record.steam_api_path).unwrap(), b"uco2-x64");
    assert_eq!(fs::read(&state_path).unwrap(), state_before);

    // Second call: known UCO2 files go, originals stay for manual recovery.
    let done = crate::online::engine::OnlineEngine::disable(
        1144200,
        &backup_root,
        &state_path,
        Some(game.path()),
        true,
    )
    .unwrap();
    assert!(done.success);
    assert!(done.warning.is_some(), "verify-integrity warning expected");
    assert!(record.backup_dir.join("original/steam_api").is_file());
    assert!(record
        .backup_dir
        .join("original/plugins/original.dll")
        .is_file());
    assert!(!game.path().join("union-crax.ini").exists());
    assert!(!game.path().join("plugins/photon_universal.dll").exists());
    // Every file left in the game plugins dir was UCO2: the empty dir goes too.
    assert!(!game.path().join("plugins").exists());
    assert!(!crate::online::foreign::scan(game.path()).uco2);
    assert!(OnlineStateStore::load(&state_path).get(1144200).is_none());
}

#[test]
fn journal_save_failure_after_restore_can_be_retried_without_deleting_original() {
    use crate::online::deploy::JournalEntry;
    let temp = tempfile::tempdir().unwrap();
    let backup_root = temp.path().join("backup");
    let backup_dir = backup_dir_for(&backup_root, 1144200);
    let state_path = temp.path().join("state.json");
    let original = temp.path().join("restored.dll");
    let backup = backup_dir.join("original/dll");
    fs::create_dir_all(backup.parent().unwrap()).unwrap();
    fs::write(&backup, b"pristine").unwrap();
    Journal {
        entries: vec![JournalEntry::BackedUp {
            original: original.clone(),
            backup: backup.clone(),
        }],
    }
    .save(&backup_dir)
    .unwrap();
    let blocked = backup_dir.join("journal.json.tmp");
    fs::create_dir(&blocked).unwrap();
    assert!(crate::online::engine::OnlineEngine::disable(
        1144200,
        &backup_root,
        &state_path,
        None,
        false
    )
    .is_err());
    assert_eq!(fs::read(&original).unwrap(), b"pristine");
    assert!(!backup.exists());
    assert!(backup_dir.join("rollback_pending.json").is_file());
    assert_eq!(Journal::load(&backup_dir).entries.len(), 1);
    fs::remove_dir(blocked).unwrap();
    assert!(
        crate::online::engine::OnlineEngine::disable(
            1144200,
            &backup_root,
            &state_path,
            None,
            false
        )
        .unwrap()
        .success
    );
    assert_eq!(fs::read(&original).unwrap(), b"pristine");
    assert!(!backup_dir.exists());
}

#[test]
fn legacy_neutralized_entry_is_still_restored() {
    use crate::online::deploy::JournalEntry;
    let temp = tempfile::tempdir().unwrap();
    let backup_root = temp.path().join("backup");
    let backup_dir = backup_dir_for(&backup_root, 1144200);
    let state_path = temp.path().join("state.json");
    let original = temp.path().join("legacy.dll");
    fs::write(
        temp.path().join("legacy.dll.uco-disabled"),
        b"legacy original",
    )
    .unwrap();
    Journal {
        entries: vec![JournalEntry::Neutralized {
            path: original.clone(),
        }],
    }
    .save(&backup_dir)
    .unwrap();
    assert!(
        crate::online::engine::OnlineEngine::disable(
            1144200,
            &backup_root,
            &state_path,
            None,
            false
        )
        .unwrap()
        .success
    );
    assert_eq!(fs::read(original).unwrap(), b"legacy original");
    assert!(!temp.path().join("legacy.dll.uco-disabled").exists());
}

#[cfg(windows)]
#[test]
fn locked_deployed_file_keeps_recovery_and_succeeds_after_unlock() {
    use std::os::windows::fs::OpenOptionsExt;
    let (game, _data, record, backup_root, state_path) = deployed_with_original_plugins();
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&record.steam_api_path)
        .unwrap();
    assert!(crate::online::engine::OnlineEngine::disable(
        1144200,
        &backup_root,
        &state_path,
        Some(game.path()),
        false
    )
    .is_err());
    assert!(record.backup_dir.join("original/steam_api").is_file());
    assert!(record.backup_dir.join("journal.json").is_file());
    assert!(OnlineStateStore::load(&state_path).get(1144200).is_some());
    drop(locked);
    assert!(
        crate::online::engine::OnlineEngine::disable(
            1144200,
            &backup_root,
            &state_path,
            Some(game.path()),
            false
        )
        .unwrap()
        .success
    );
    assert_eq!(fs::read(&record.steam_api_path).unwrap(), b"original-dll");
}
