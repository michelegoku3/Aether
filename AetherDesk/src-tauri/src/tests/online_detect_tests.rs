//! Test dell'engine di rilevamento UCOnline2 (`online::detect`).
//!
//! Le fixture sono alberi di cartelle fittizi creati su `tempdir`: nessun
//! gioco reale, nessuna dipendenza da Steam o Tauri.
//!
//! NOTA: questo file è condiviso tra la repo (incluso da `tests/mod.rs`) e
//! l'harness di verifica standalone (`uco2_engine_harness`, incluso via
//! `#[path]`): non cambiare i path di import senza aggiornare entrambi.

use crate::online::detect::{DetectionError, GameInspector};
use crate::online::types::{Conflict, Engine, GameArch, PhotonFlavor};
use std::fs;
use std::path::{Component, Path};

fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// True quando gli ultimi componenti di `path` sono esattamente `comps`
/// (cross-platform: niente confronti su stringhe con `/` o `\`).
fn ends_with_components(path: &Path, comps: &[&str]) -> bool {
    let actual: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let start = actual.len().saturating_sub(comps.len());
    actual.len() >= comps.len()
        && actual[start..]
            .iter()
            .zip(comps)
            .all(|(a, b)| a.as_str() == *b)
}

/// Un mini-gioco Unity Mono: `<Game>.exe` + `<Game>_Data/Managed/…`.
fn unity_mono_fixture(root: &Path) {
    write(root, "MyGame.exe", b"MZfakeexe");
    write(root, "MyGame_Data/Managed/Assembly-CSharp.dll", b"assembly");
    write(root, "MyGame_Data/Plugins/x86_64/steam_api64.dll", b"dll");
}

#[test]
fn unity_game_is_detected() {
    let tmp = tempfile::tempdir().unwrap();
    unity_mono_fixture(tmp.path());

    let report = GameInspector::inspect(tmp.path()).expect("Unity game must be detected");
    assert_eq!(report.engine, Engine::Unity);
    assert_eq!(report.arch, GameArch::X64);
    assert_eq!(report.ini_dir, tmp.path());
    // La DLL esiste già in <Data>\Plugins\x86_64\ → è la posizione autoritativa.
    assert!(ends_with_components(
        &report.steam_api_dir.unwrap(),
        &["MyGame_Data", "Plugins", "x86_64"]
    ));
    assert_eq!(
        report
            .game_exe
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .as_deref(),
        Some("MyGame.exe".into())
    );
    assert!(ends_with_components(&report.unity_data_dir.unwrap(), &["MyGame_Data"]));
}

#[test]
fn unity_without_marker_is_not_unity() {
    // `web_data` di Farming Simulator: *_Data senza Managed né il2cpp.
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "Game/web_data/whatever.bin", b"x");
    write(tmp.path(), "Game/Game.exe", b"MZ");
    write(tmp.path(), "Game/steam_api64.dll", b"dll");

    let report = GameInspector::inspect(tmp.path()).expect("must fall back to Generic");
    assert_eq!(report.engine, Engine::Generic);
}

#[test]
fn unity_without_any_dll_uses_plugins_convention() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "MyGame.exe", b"MZ");
    write(tmp.path(), "MyGame_Data/Managed/Assembly-CSharp.dll", b"a");

    let report = GameInspector::inspect(tmp.path()).unwrap();
    assert_eq!(report.engine, Engine::Unity);
    // Fallback convenzione: <Data>\Plugins\x86_64\
    assert!(ends_with_components(
        &report.steam_api_dir.unwrap(),
        &["MyGame_Data", "Plugins", "x86_64"]
    ));
}

#[test]
fn unreal_shipping_exe_found_and_crash_report_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "Engine/Binaries/Win64/MyGame-Win64-Shipping.exe",
        b"MZshipping",
    );
    // CrashReportClient NON deve vincere.
    write(
        tmp.path(),
        "Engine/Binaries/Win64/CrashReportClient-Win64-Shipping.exe",
        b"MZcrash",
    );
    // Posizione tipica della DLL per Unreal.
    write(
        tmp.path(),
        "Engine/Binaries/ThirdParty/Steamworks/Steamv153/Win64/steam_api64.dll",
        b"dll",
    );

    let report = GameInspector::inspect(tmp.path()).unwrap();
    assert_eq!(report.engine, Engine::Unreal);
    assert!(ends_with_components(
        &report.game_exe.unwrap(),
        &["Engine", "Binaries", "Win64", "MyGame-Win64-Shipping.exe"]
    ));
    // ini va accanto all'exe in esecuzione, NON nella root!
    assert!(ends_with_components(&report.ini_dir, &["Engine", "Binaries", "Win64"]));
    assert!(ends_with_components(
        &report.steam_api_dir.unwrap(),
        &["ThirdParty", "Steamworks", "Steamv153", "Win64"]
    ));
}

#[test]
fn duplicate_steam_api64_picks_dir_next_to_biggest_exe() {
    let tmp = tempfile::tempdir().unwrap();
    // Copia in root accanto a un launcher piccolo...
    write(tmp.path(), "MyGame.exe", b"MZsmalllauncher");
    write(tmp.path(), "steam_api64.dll", b"root-copy");
    // ...e copia in x64\ accanto al binario vero (più grande).
    write(tmp.path(), "x64/MyGame.exe", b"MZthis-is-the-big-real-game-binary");
    write(tmp.path(), "x64/steam_api64.dll", b"real-copy");

    let report = GameInspector::inspect(tmp.path()).unwrap();
    assert!(ends_with_components(&report.steam_api_dir.unwrap(), &["x64"]));
}

#[test]
fn nested_repack_unity_is_found_recursively() {
    // Layout ColdClientLoader: loader in root, gioco vero annidato.
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "Vampire Survivors.exe", b"MZloader");
    write(tmp.path(), "Vampire Survivors/VampireSurvivors.exe", b"MZgame");
    write(
        tmp.path(),
        "Vampire Survivors/VampireSurvivors_Data/Managed/Assembly-CSharp.dll",
        b"a",
    );

    let report = GameInspector::inspect(tmp.path()).unwrap();
    assert_eq!(report.engine, Engine::Unity);
    // ini_dir = parent della *_Data (la dir del gioco annidato).
    assert!(ends_with_components(&report.ini_dir, &["Vampire Survivors"]));
}

#[test]
fn x86_game_is_detected() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "OldGame.exe", b"MZ");
    write(tmp.path(), "OldGame_Data/Managed/Assembly-CSharp.dll", b"a");
    write(tmp.path(), "OldGame_Data/Plugins/x86/steam_api.dll", b"dll32");

    let report = GameInspector::inspect(tmp.path()).unwrap();
    assert_eq!(report.arch, GameArch::X86);
    assert!(ends_with_components(
        &report.steam_api_dir.unwrap(),
        &["OldGame_Data", "Plugins", "x86"]
    ));
    // La DLL da installare per questo gioco è quella x86.
    assert_eq!(report.arch.steam_api_file_name(), "steam_api.dll");
}

#[test]
fn mono_backends_photon_voice_playfab() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "MyGame.exe", b"MZ");
    write(tmp.path(), "MyGame_Data/Managed/Assembly-CSharp.dll", b"a");
    write(tmp.path(), "MyGame_Data/Managed/PhotonRealtime.dll", b"photon");
    write(tmp.path(), "MyGame_Data/Managed/PhotonVoice.dll", b"voice");
    write(tmp.path(), "MyGame_Data/Managed/PlayFabAllSDK.dll", b"playfab");

    let report = GameInspector::inspect(tmp.path()).unwrap();
    assert_eq!(report.backends.photon, PhotonFlavor::Realtime);
    assert!(report.backends.photon_voice);
    assert!(report.backends.playfab);
    assert!(!report.backends.eos);
    assert!(!report.backends.coherence);
    assert_eq!(
        report.backends.required_plugins(),
        vec!["photon_universal", "playfab_universal"]
    );
}

#[test]
fn mono_fusion_flavor() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "MyGame.exe", b"MZ");
    write(tmp.path(), "MyGame_Data/Managed/Fusion.Realtime.dll", b"fusion");

    let report = GameInspector::inspect(tmp.path()).unwrap();
    assert_eq!(report.backends.photon, PhotonFlavor::Fusion);
}

#[test]
fn il2cpp_backends_from_metadata_strings() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "MyGame.exe", b"MZ");
    write(tmp.path(), "MyGame_Data/il2cpp_data/Metadata/global-metadata.dat", b"garbage LoadBalancingClient garbage PlayFabSettings CoherenceBridge");

    let report = GameInspector::inspect(tmp.path()).unwrap();
    assert_eq!(report.backends.photon, PhotonFlavor::Realtime);
    assert!(report.backends.playfab);
    assert!(report.backends.coherence);
}

#[test]
fn unreal_eos_from_exe_and_sdk_dll() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "Engine/Binaries/Win64/MyGame-Win64-Shipping.exe",
        b"header OnlineSubsystemEOS footer",
    );
    write(
        tmp.path(),
        "Engine/Binaries/Win64/EOSSDK-Win64-Shipping.dll",
        b"eos sdk",
    );

    let report = GameInspector::inspect(tmp.path()).unwrap();
    assert_eq!(report.engine, Engine::Unreal);
    assert!(report.backends.eos);
}

#[test]
fn coherence_schema_file_is_a_marker() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "MyGame.exe", b"MZ");
    write(tmp.path(), "MyGame_Data/Managed/Assembly-CSharp.dll", b"a");
    write(tmp.path(), "MyGame_Data/StreamingAssets/combined.schema", b"{}");

    let report = GameInspector::inspect(tmp.path()).unwrap();
    assert!(report.backends.coherence);
}

#[test]
fn conflicts_are_detected() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "Game.exe", b"MZ");
    write(tmp.path(), "steamclient64.ini", b"[settings]");
    write(tmp.path(), "steam_api64.dll", b"dll");
    write(tmp.path(), "SteamFix64.dll", b"fix");
    write(tmp.path(), "winmm.dll", b"tiny-proxy");
    // Proxy grande → NON è un conflitto (file di gioco legittimo).
    let big = vec![0u8; 500_000];
    write(tmp.path(), "dxgi.dll", &big);

    let report = GameInspector::inspect(tmp.path()).unwrap();
    let kinds: Vec<String> = report
        .conflicts
        .iter()
        .map(|c| match c {
            Conflict::ColdClientLoader(_) => "coldclient".to_string(),
            Conflict::SteamFix(_) => "steamfix".to_string(),
            Conflict::OnlineFix(_) => "onlinefix".to_string(),
            Conflict::NamedFixFile(_) => "named".to_string(),
            Conflict::ProxyDll(_) => "proxy".to_string(),
        })
        .collect();
    assert!(kinds.contains(&"coldclient".to_string()));
    assert!(kinds.contains(&"steamfix".to_string()));
    assert!(kinds.contains(&"named".to_string()));
    assert!(!kinds.contains(&"proxy".to_string()), "big dxgi.dll must not be a proxy conflict");
}

#[test]
fn steamless_markers_are_detected() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "Game.exe", b"MZ");
    write(tmp.path(), "steam_api64.dll", b"dll");
    write(tmp.path(), "Game.exe.steamstub.bak", b"orig");

    let report = GameInspector::inspect(tmp.path()).unwrap();
    assert!(report.steamless_applied);
}

#[test]
fn unrecognized_game_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "readme.txt", b"not a game");

    let err = GameInspector::inspect(tmp.path()).unwrap_err();
    assert!(matches!(err, DetectionError::UnrecognizedGame(_)));
}

#[test]
fn x86_game_has_warning() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "OldGame.exe", b"MZ");
    write(tmp.path(), "OldGame_Data/Managed/Assembly-CSharp.dll", b"a");
    write(tmp.path(), "OldGame_Data/Plugins/x86/steam_api.dll", b"dll32");

    let report = GameInspector::inspect(tmp.path()).unwrap();
    assert!(report
        .warnings
        .iter()
        .any(|w| w.contains("32 bit")));
}
