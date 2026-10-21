//! `--apply-update` must validate the install root before it writes anything.

use std::fs;
use std::path::PathBuf;

/// Isolated install root; the tag keeps parallel tests apart.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "aether_updater_apply_tests_{}_{}",
        std::process::id(),
        tag
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn apply_update_refuses_root_without_matching_exe() {
    let root = temp_dir("apply-refuse");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("AetherDesk.exe"), b"not the running binary").unwrap();
    let err = crate::updater::desk::validate_install_root(&root).unwrap_err();
    assert!(err.contains("AetherDesk.exe"), "{err}");
}

#[test]
fn apply_update_refuses_protected_root() {
    let err = crate::updater::desk::validate_install_root(&dirs::home_dir().unwrap()).unwrap_err();
    assert!(!err.is_empty());
}
