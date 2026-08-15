//! Test del bundle UCOnline2 (`online::bundle`).
//!
//! NOTA: file condiviso tra la repo (incluso da `tests/mod.rs`) e l'harness
//! standalone — non cambiare i path di import senza aggiornare entrambi.

use crate::online::bundle::Uco2Bundle;
use crate::online::types::GameArch;
use std::fs;
use std::path::Path;

fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// Fixture con il layout minimo valido della release ufficiale.
fn valid_bundle_fixture(dir: &Path) {
    write(dir, "x64/steam_api64.dll", b"dll64");
    write(dir, "x86/steam_api.dll", b"dll32");
    write(dir, "plugins/photon_universal.dll", b"plugin");
    write(dir, "plugins/EOS_custom.dll", b"plugin");
    write(dir, "plugins/coherence_universal.dll", b"plugin");
}

#[test]
fn valid_bundle_is_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    valid_bundle_fixture(tmp.path());

    let bundle = Uco2Bundle::open(tmp.path().to_path_buf()).expect("valid bundle");
    assert_eq!(bundle.dir(), tmp.path());
}

#[test]
fn incomplete_bundle_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    // Manca x86/ e plugins/ → invalido.
    write(tmp.path(), "x64/steam_api64.dll", b"dll64");

    assert!(!Uco2Bundle::is_valid_dir(tmp.path()));
    assert!(Uco2Bundle::open(tmp.path().to_path_buf()).is_err());
}

#[test]
fn empty_plugins_dir_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "x64/steam_api64.dll", b"dll64");
    write(tmp.path(), "x86/steam_api.dll", b"dll32");
    fs::create_dir_all(tmp.path().join("plugins")).unwrap();

    assert!(!Uco2Bundle::is_valid_dir(tmp.path()));
}

#[test]
fn steam_api_dll_resolution_per_arch() {
    let tmp = tempfile::tempdir().unwrap();
    valid_bundle_fixture(tmp.path());
    let bundle = Uco2Bundle::open(tmp.path().to_path_buf()).unwrap();

    assert!(bundle
        .steam_api_dll(GameArch::X64)
        .ends_with("x64/steam_api64.dll"));
    assert!(bundle
        .steam_api_dll(GameArch::X86)
        .ends_with("x86/steam_api.dll"));
}

#[test]
fn plugin_lookup_is_case_insensitive() {
    let tmp = tempfile::tempdir().unwrap();
    valid_bundle_fixture(tmp.path());
    let bundle = Uco2Bundle::open(tmp.path().to_path_buf()).unwrap();

    let found = bundle
        .plugin_dll("eos_custom")
        .expect("plugin found case-insensitively");
    assert!(found.ends_with("EOS_custom.dll"));

    assert!(bundle.plugin_dll("coherence_universal").is_some());
    assert!(bundle.plugin_dll("nonexistent").is_none());
}

#[test]
fn version_file_is_read() {
    let tmp = tempfile::tempdir().unwrap();
    valid_bundle_fixture(tmp.path());
    write(tmp.path(), "VERSION", b"v1.19.3\n");

    let bundle = Uco2Bundle::open(tmp.path().to_path_buf()).unwrap();
    assert_eq!(bundle.version().as_deref(), Some("v1.19.3"));

    // Senza VERSION → None, non errore.
    let tmp2 = tempfile::tempdir().unwrap();
    valid_bundle_fixture(tmp2.path());
    let bundle2 = Uco2Bundle::open(tmp2.path().to_path_buf()).unwrap();
    assert_eq!(bundle2.version(), None);
}
