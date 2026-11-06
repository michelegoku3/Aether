//! The install root must be validated before the update writes anything: in
//! the parent, so the refusal reaches the user instead of the app silently
//! closing, and in the `--apply-update` process, as the safety net for a
//! manually invoked apply.

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

/// Creates a directory junction without requiring administrator rights.
/// Returns false when the platform refuses (the test then has nothing to prove).
#[cfg(windows)]
fn make_junction(link: &std::path::Path, target: &std::path::Path) -> bool {
    std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// A redirected Desktop/Documents (OneDrive Known Folder Move) puts a reparse
/// point above a perfectly valid portable install. Only the install root and
/// the exe it replaces are the updater's business.
#[cfg(windows)]
#[test]
fn an_ancestor_reparse_point_does_not_block_a_valid_install_root() {
    let base = temp_dir("ancestor-junction");
    let real = base.join("real");
    let install = real.join("install");
    fs::create_dir_all(&install).unwrap();
    fs::copy(
        std::env::current_exe().unwrap(),
        install.join("AetherDesk.exe"),
    )
    .unwrap();
    let link = base.join("link");
    if !make_junction(&link, &real) {
        return;
    }

    let root = crate::updater::desk::validate_install_root(&link.join("install")).unwrap();

    assert_eq!(root, fs::canonicalize(&install).unwrap());
}

/// The guard still refuses an install root that is itself a link: writing
/// through it would replace files outside the real installation.
#[cfg(windows)]
#[test]
fn a_reparse_point_at_the_install_root_is_refused() {
    let base = temp_dir("root-junction");
    let real = base.join("real");
    fs::create_dir_all(&real).unwrap();
    fs::copy(
        std::env::current_exe().unwrap(),
        real.join("AetherDesk.exe"),
    )
    .unwrap();
    let link = base.join("link");
    if !make_junction(&link, &real) {
        return;
    }

    let err = crate::updater::desk::validate_install_root(&link).unwrap_err();

    assert!(err.contains("linked/reparse"), "{err}");
}

/// The parent refuses an invalid install root before it stages an updater or
/// spawns anything, so the user sees the reason instead of a closing app.
#[test]
fn schedule_restart_refuses_an_install_root_without_a_matching_exe() {
    let root = temp_dir("restart-refuse");
    fs::write(root.join("AetherDesk.exe"), b"not the running binary").unwrap();
    let prepared = crate::updater::desk::PreparedUpdate {
        app_root: root.clone(),
        install_root: root,
    };

    let err = crate::updater::desk::schedule_restart(&prepared).unwrap_err();

    assert!(err.contains("AetherDesk.exe"), "{err}");
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

#[test]
fn apply_helper_logs_beside_the_install_root_data() {
    let root = temp_dir("apply-log-root");
    assert_eq!(
        crate::core::logger::log_file_for_root(&root),
        root.join("AetherData").join("logs").join("desk.log")
    );
}

/// The wait for a locked exe must respect its budget and stop, so the helper
/// fails visibly instead of blocking the update forever.
#[cfg(windows)]
#[test]
fn a_locked_exe_is_not_reported_replaceable() {
    use std::os::windows::fs::OpenOptionsExt;
    use std::time::{Duration, Instant};

    let root = temp_dir("wait-locked");
    let exe = root.join("AetherDesk.exe");
    fs::write(&exe, b"binary").unwrap();

    // share_mode(0) makes Windows refuse the rename the probe performs.
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&exe)
        .expect("lock the exe");

    let started = Instant::now();
    assert!(
        !crate::updater::desk::wait_until_replaceable(&exe, Duration::from_millis(300)),
        "a locked exe is replaceable only after the lock goes away"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the budget must end the wait, took {:?}",
        started.elapsed()
    );

    drop(lock);

    assert!(crate::updater::desk::wait_until_replaceable(
        &exe,
        Duration::from_secs(5)
    ));
    assert!(exe.is_file(), "the probe must leave the exe in place");
    let _ = fs::remove_dir_all(root);
}

/// A merge that fails halfway must leave the previously installed exe in
/// place: the helper relaunches it, so a broken update is a visible failure
/// instead of an app that closed and never came back.
#[test]
fn a_failed_merge_restores_the_original_exe() {
    let root = temp_dir("merge-restore");
    let source = std::env::current_exe().unwrap();
    fs::copy(&source, root.join("AetherDesk.exe")).unwrap();
    // The staged tree carries a *file* where the installation has a directory:
    // `fs::copy` cannot overwrite a directory, so the merge must fail.
    fs::create_dir_all(root.join("tools")).unwrap();

    let staging = temp_dir("merge-restore-staging");
    fs::write(staging.join("tools"), b"staged tools file").unwrap();
    fs::write(staging.join("AetherDesk.exe"), b"new exe").unwrap();

    let error = crate::updater::desk::apply_update(&staging, &root).unwrap_err();

    assert!(error.contains("tools"), "unexpected error: {error}");
    assert!(
        root.join("AetherDesk.exe").is_file(),
        "the previous exe must be back in place"
    );
    assert_eq!(
        fs::read(root.join("AetherDesk.exe")).unwrap(),
        fs::read(&source).unwrap(),
        "the restored exe must be the previous build, not the staged one"
    );
    assert!(
        !root.join("AetherDesk.exe.old").exists(),
        "no leftover backup of the exe"
    );
}
