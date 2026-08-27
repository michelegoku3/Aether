use crate::online::foreign::{ForeignOnlineReport, has_ofme, scan};
use crate::online::types::Conflict;
use std::fs;
use std::path::{Path, PathBuf};

fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
fn onlinefix64_is_ofme() {
    let conflicts = vec![Conflict::OFME(PathBuf::from("C:/game/OnlineFix64.dll"))];
    assert!(has_ofme(&conflicts));
    let report = ForeignOnlineReport::from_conflicts(&conflicts);
    assert!(report.ofme);
    assert!(report.refuse_uco2().contains("OnlineFix64.dll"));
}

#[test]
fn steamfix_is_ofme_family() {
    let conflicts = vec![Conflict::SteamFix(PathBuf::from("SteamFix64.dll"))];
    assert!(has_ofme(&conflicts));
}

#[test]
fn proxy_dll_alone_is_not_ofme() {
    let conflicts = vec![Conflict::ProxyDll(PathBuf::from("dxgi.dll"))];
    assert!(!has_ofme(&conflicts));
}

#[test]
fn winmm_without_sibling_is_not_ofme() {
    let conflicts = vec![Conflict::NamedFixFile(PathBuf::from("/tmp/no-such-game/winmm.dll"))];
    assert!(!has_ofme(&conflicts));
}

#[test]
fn dlllist_named_file_is_ofme() {
    let conflicts = vec![Conflict::NamedFixFile(PathBuf::from("dlllist.txt"))];
    assert!(has_ofme(&conflicts));
}

#[test]
fn nested_ofme_pack_next_to_shipping_exe_is_detected() {
    // Bodycam-style: steam_api in ThirdParty, OFME accanto al Shipping exe.
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "Engine/Binaries/Win64/Bodycam-Win64-Shipping.exe",
        b"MZ",
    );
    write(
        tmp.path(),
        "Engine/Binaries/ThirdParty/Steamworks/Win64/steam_api64.dll",
        b"dll",
    );
    write(
        tmp.path(),
        "Engine/Binaries/Win64/OnlineFix64.dll",
        b"ofme",
    );
    write(tmp.path(), "Engine/Binaries/Win64/OnlineFix.ini", b"[fix]");

    let report = scan(tmp.path());
    assert!(report.ofme, "nested OnlineFix64.dll must be detected");
    assert!(!report.uco2);
    assert!(report
        .files
        .iter()
        .any(|p| p.file_name().and_then(|n| n.to_str()) == Some("OnlineFix64.dll")));
}

#[test]
fn nested_union_crax_is_uco2_not_ofme() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "Game.exe", b"MZ");
    write(tmp.path(), "Binaries/Win64/union-crax.ini", b"[Settings]");
    write(tmp.path(), "Binaries/Win64/steam_api64.dll", b"dll");

    let report = scan(tmp.path());
    assert!(report.uco2);
    assert!(!report.ofme);
}

#[test]
fn overlay_proxy_dll_is_uco2() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "overlay_proxy.dll", b"proxy");

    let report = scan(tmp.path());
    assert!(report.uco2);
    assert!(!report.ofme);
}

#[test]
fn winmm_alone_on_disk_is_not_ofme() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "winmm.dll", b"audio");

    let report = scan(tmp.path());
    assert!(!report.ofme);
}

#[test]
fn sweep_removes_union_crax_not_steam_api() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "Binaries/Win64/union-crax.ini", b"[Settings]");
    write(tmp.path(), "Binaries/Win64/steam_api64.dll", b"dll");

    let removed = crate::online::foreign::sweep_uco2_files(tmp.path());
    assert_eq!(removed, 1);
    assert!(!tmp.path().join("Binaries/Win64/union-crax.ini").is_file());
    assert!(tmp.path().join("Binaries/Win64/steam_api64.dll").is_file());
}

#[test]
fn content_dir_is_not_scanned() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "Content/OnlineFix64.dll", b" decoy ");
    write(tmp.path(), "Game.exe", b"MZ");

    let report = scan(tmp.path());
    assert!(!report.ofme, "asset folders must not produce OFME hits");
}
