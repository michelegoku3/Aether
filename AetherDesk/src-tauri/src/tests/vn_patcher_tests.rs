//! Tests for launching a VN patch executable that is not a self-extracting
//! archive.
//!
//! The pipeline may only count the patch as applied when the installer really
//! ran and exited successfully; these tests pin the two failure paths — a
//! non-zero exit code and an installer that cannot be started at all.

use crate::crack::{installer_exit_result, launch_patch_installer};
use std::fs;
use std::path::{Path, PathBuf};

/// Scratch directory in the system temp folder, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `tag` keeps concurrent tests of the same process apart.
fn temp_dir(tag: &str) -> TempDir {
    let path = std::env::temp_dir().join(format!(
        "aetherdesk_vn_patch_{}_{}",
        std::process::id(),
        tag
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    TempDir(path)
}

/// A patch that runs but fails must not be reported as applied.
#[cfg(windows)]
#[test]
fn nonzero_exit_is_reported_as_failure() {
    let status = std::process::Command::new("cmd.exe")
        .args(["/c", "exit", "3"])
        .status()
        .expect("spawn cmd.exe");
    let err = installer_exit_result(status).unwrap_err();
    assert!(err.contains('3'), "{err}");
}

/// A patch that cannot be started at all must surface as a failure instead of
/// a silent success.
#[cfg(windows)]
#[test]
fn missing_installer_is_reported_as_failure() {
    let dir = temp_dir("missing");
    let missing = dir.path().join("nope.exe");
    assert!(launch_patch_installer(&missing, dir.path()).is_err());
}
