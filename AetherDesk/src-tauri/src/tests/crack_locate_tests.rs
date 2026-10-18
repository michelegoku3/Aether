//! Unit tests for crack target resolution (locate.rs).
//!
//! Pins support for:
//! - Hierarchical cracks (folders): keep the usual suffix-matching.
//! - Flat cracks (`is_flat_crack = true`): recursive search and replacement
//!   of every matching file in any subfolder.
//! - Counterpart-free files: the fallback base is the game's real root, which
//!   for a wrapper-packaged install (`common/<name>/<name>/game.exe`) is the
//!   inner folder, not Steam's `installdir` folder.

use crate::crack::locate::{resolve_file_target, resolve_file_targets};
use std::fs;
use std::path::{Path, PathBuf};

/// The reported shape: Steam's `installdir` ("How to Fish") holds nothing but
/// a folder with the same name, and the game lives inside that folder.
const WRAPPER: &str = "How to Fish";

/// Creates the real game (Unity layout: exe in the inner folder, the Steam API
/// DLL under `<Game>_Data\Plugins\x86_64`) and returns the inner folder.
fn wrapped_game(root: &Path) -> PathBuf {
    let inner = root.join(WRAPPER);
    let plugins = inner
        .join("How to Fish_Data")
        .join("Plugins")
        .join("x86_64");
    fs::create_dir_all(&plugins).unwrap();
    fs::write(inner.join("How to Fish.exe"), b"MZ fake").unwrap();
    fs::write(plugins.join("steam_api64.dll"), b"pristine").unwrap();
    inner
}

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

/// A crack file that exists nowhere in the game belongs next to the game, not
/// in Steam's wrapper folder: `common/<name>/<name>/game.exe` means the file
/// lands in the inner `<name>`.
#[test]
fn test_counterpart_free_flat_crack_lands_in_the_wrapped_game_root() {
    let temp_dir = tempfile_tempdir();
    let game_root = temp_dir.path();
    let inner = wrapped_game(game_root);

    let rel = Path::new("OnlineFix.ini");
    let targets = resolve_file_targets(rel, game_root, true);

    assert_eq!(targets, vec![inner.join("OnlineFix.ini")]);
}

/// Same rule for a crack archive that carries a folder: the folder is resolved
/// against the real game root, so it merges where the game actually keeps it.
#[test]
fn test_counterpart_free_folder_crack_merges_into_the_wrapped_game_root() {
    let temp_dir = tempfile_tempdir();
    let game_root = temp_dir.path();
    let inner = wrapped_game(game_root);
    fs::create_dir_all(inner.join("Binaries")).unwrap();

    let rel = Path::new("Binaries").join("extra.dll");
    let targets = resolve_file_targets(&rel, game_root, false);

    assert_eq!(targets, vec![inner.join("Binaries").join("extra.dll")]);
}

/// Wrapper folders can be nested more than once; the descent follows them.
#[test]
fn test_counterpart_free_file_reaches_a_doubly_wrapped_game_root() {
    let temp_dir = tempfile_tempdir();
    let game_root = temp_dir.path();
    let inner = game_root.join("A").join("A");
    fs::create_dir_all(&inner).unwrap();
    fs::write(inner.join("Game.exe"), b"MZ fake").unwrap();

    let rel = Path::new("OnlineFix.ini");
    let targets = resolve_file_targets(rel, game_root, true);

    assert_eq!(targets, vec![inner.join("OnlineFix.ini")]);
}

/// Guard: when the installdir holds anything besides a single folder, the game
/// may genuinely live there, so the base must stay the installdir itself.
#[test]
fn test_wrapper_descent_stops_when_the_installdir_holds_other_files() {
    let temp_dir = tempfile_tempdir();
    let game_root = temp_dir.path();
    let inner = wrapped_game(game_root);
    fs::write(game_root.join("readme.txt"), b"shipped with the game").unwrap();

    let rel = Path::new("OnlineFix.ini");
    let targets = resolve_file_targets(rel, game_root, true);

    assert_eq!(targets, vec![game_root.join("OnlineFix.ini")]);
    assert_ne!(targets, vec![inner.join("OnlineFix.ini")]);
}

/// Characterisation of the one shape the wrapper rule deliberately does not
/// cover, plus the documented recovery: a file next to the wrapper folder keeps
/// the base at the installdir (nothing can tell a crack file left by an older
/// build apart from a file the game owns), and removing that file restores the
/// rule - which is exactly what "remove applied crack" and then applying again
/// does for a game cracked by an older AetherDesk.
#[test]
fn test_stray_file_at_the_installdir_keeps_the_base_until_it_is_removed() {
    let temp_dir = tempfile_tempdir();
    let game_root = temp_dir.path();
    let inner = wrapped_game(game_root);
    let stray = game_root.join("OnlineFix.ini");
    fs::write(&stray, b"[crack left by an older build]").unwrap();

    let rel = Path::new("steam_appid.txt");
    assert_eq!(
        resolve_file_targets(rel, game_root, true),
        vec![game_root.join("steam_appid.txt")]
    );

    fs::remove_file(&stray).unwrap();
    assert_eq!(
        resolve_file_targets(rel, game_root, true),
        vec![inner.join("steam_appid.txt")]
    );
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
    let path = std::env::temp_dir().join(format!(
        "aetherdesk_crack_locate_test_{}_{}",
        std::process::id(),
        id
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    TempDirGuard(path)
}
