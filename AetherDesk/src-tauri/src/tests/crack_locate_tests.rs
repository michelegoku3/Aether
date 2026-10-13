//! Unit tests for crack target resolution (locate.rs).
//!
//! Pins support for:
//! - Hierarchical cracks (folders): keep the usual suffix-matching.
//! - Flat cracks (`is_flat_crack = true`): recursive search and replacement
//!   of every matching file in any subfolder, or fallback to the root when
//!   no file matches.

use crate::crack::locate::{resolve_file_target, resolve_file_targets};
use std::fs;
use std::path::Path;

#[test]
fn test_resolve_file_targets_hierarchical_crack() {
    let temp_dir = tempfile_tempdir();
    let game_root = temp_dir.path();

    // With is_flat_crack = false, use the standard logic
    let rel = Path::new("files").join("Binaries").join("steam_api64.dll");
    let targets = resolve_file_targets(&rel, game_root, false);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0], resolve_file_target(&rel, game_root));
}

#[test]
fn test_resolve_file_targets_flat_crack_no_match_falls_back_to_root() {
    let temp_dir = tempfile_tempdir();
    let game_root = temp_dir.path();

    let rel = Path::new("OnlineFix.ini");
    let targets = resolve_file_targets(rel, game_root, true);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0], game_root.join("OnlineFix.ini"));
}

#[test]
fn test_resolve_file_targets_flat_crack_matches_subdirs() {
    let temp_dir = tempfile_tempdir();
    let game_root = temp_dir.path();

    // Create two identical files in two different subfolders
    let dir1 = game_root.join("Binaries").join("Win64");
    let dir2 = game_root.join("Engine").join("Binaries");
    fs::create_dir_all(&dir1).unwrap();
    fs::create_dir_all(&dir2).unwrap();

    let file1 = dir1.join("steam_api64.dll");
    let file2 = dir2.join("steam_api64.dll");
    fs::write(&file1, "test1").unwrap();
    fs::write(&file2, "test2").unwrap();

    let rel = Path::new("steam_api64.dll");
    let mut targets = resolve_file_targets(rel, game_root, true);
    targets.sort();

    let mut expected = vec![file1, file2];
    expected.sort();

    assert_eq!(targets, expected);
}

/// Fallback helper for direct tests with no extra dependencies.
struct TempDirGuard(std::path::PathBuf);
impl TempDirGuard {
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tempfile_tempdir() -> TempDirGuard {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("aetherdesk_crack_locate_test_{}_{}", std::process::id(), id));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    TempDirGuard(path)
}
