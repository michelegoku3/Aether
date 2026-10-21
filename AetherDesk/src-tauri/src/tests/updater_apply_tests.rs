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
    let home = dirs::home_dir().unwrap();
    let err = crate::updater::desk::validate_install_root(&home).unwrap_err();
    // The protected-folder guard answers before the exe-hash guard, so the
    // message names the protected root and not a mismatching executable.
    assert!(err.contains("Refusing protected uninstall root"), "{err}");
}

#[test]
fn staged_updater_lives_in_a_fresh_directory() {
    let a = crate::updater::desk::stage_updater_for_test().unwrap();
    let b = crate::updater::desk::stage_updater_for_test().unwrap();
    let (first_dir, second_dir) = (
        a.parent().unwrap().to_path_buf(),
        b.parent().unwrap().to_path_buf(),
    );

    assert_ne!(first_dir, second_dir);
    assert!(a.is_file());

    let _ = fs::remove_dir_all(first_dir);
    let _ = fs::remove_dir_all(second_dir);
}

#[test]
fn staged_updater_hash_matches_source() {
    let staged = crate::updater::desk::stage_updater_for_test().unwrap();
    let staged_dir = staged.parent().unwrap().to_path_buf();

    assert_eq!(
        crate::updater::desk::hash_file(&staged).unwrap(),
        crate::updater::desk::hash_file(&std::env::current_exe().unwrap()).unwrap()
    );

    let _ = fs::remove_dir_all(staged_dir);
}
