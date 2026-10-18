//! Unit tests for crack target resolution (locate.rs).
//!
//! Pins support for:
//! - Hierarchical cracks (folders): keep the usual suffix-matching.
//! - Flat cracks (`is_flat_crack = true`): recursive search and replacement
//!   of every matching file in any subfolder.
//! - Counterpart-free files: the fallback base is the game's real root, which
//!   for a wrapper-packaged install (`common/<name>/<name>/game.exe`) is the
//!   inner folder, not Steam's `installdir` folder. The descent needs a signal
//!   (the only entry, a name equal to the installdir, or the game's own
//!   executable inside) and stops when the installdir itself holds one.

use crate::crack::locate::{resolve_file_target, resolve_file_targets};
use std::fs;
use std::path::{Path, PathBuf};

/// The reported shape: Steam creates `steamapps/common/<name>/` (`<name>` is the
/// ACF `installdir`, here "How to Fish") and the real game lives in a folder
/// with the same name inside it.
const WRAPPER: &str = "How to Fish";

/// Builds that install under `common` and returns `(installdir, real game
/// folder)`. The game is a Unity x64 layout: the exe sits in the inner folder
/// and the Steam API DLL under `<Game>_Data\Plugins\x86_64`.
fn wrapped_game(common: &Path) -> (PathBuf, PathBuf) {
    let installdir = common.join(WRAPPER);
    let inner = installdir.join(WRAPPER);
    let plugins = inner
        .join("How to Fish_Data")
        .join("Plugins")
        .join("x86_64");
    fs::create_dir_all(&plugins).unwrap();
    fs::write(inner.join("How to Fish.exe"), b"MZ fake").unwrap();
    fs::write(plugins.join("steam_api64.dll"), b"pristine").unwrap();
    (installdir, inner)
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
    let (game_root, inner) = wrapped_game(temp_dir.path());

    let rel = Path::new("OnlineFix.ini");
    let targets = resolve_file_targets(rel, &game_root, true);

    assert_eq!(targets, vec![inner.join("OnlineFix.ini")]);
}

/// Same rule for a crack archive that carries a folder: the folder is resolved
/// against the real game root, so it merges where the game actually keeps it.
#[test]
fn test_counterpart_free_folder_crack_merges_into_the_wrapped_game_root() {
    let temp_dir = tempfile_tempdir();
    let (game_root, inner) = wrapped_game(temp_dir.path());
    fs::create_dir_all(inner.join("Binaries")).unwrap();

    let rel = Path::new("Binaries").join("extra.dll");
    let targets = resolve_file_targets(&rel, &game_root, false);

    assert_eq!(targets, vec![inner.join("Binaries").join("extra.dll")]);
}

/// Wrapper folders can be nested more than once; the descent follows them.
#[test]
fn test_counterpart_free_file_reaches_a_doubly_wrapped_game_root() {
    let temp_dir = tempfile_tempdir();
    let game_root = temp_dir.path().join("A");
    let inner = game_root.join("A");
    fs::create_dir_all(&inner).unwrap();
    fs::write(inner.join("Game.exe"), b"MZ fake").unwrap();

    let rel = Path::new("OnlineFix.ini");
    let targets = resolve_file_targets(rel, &game_root, true);

    assert_eq!(targets, vec![inner.join("OnlineFix.ini")]);
}

/// A stray file next to the wrapper (a repack note, or a crack file left by an
/// older build) is not enough to give up: the folder is named like the
/// installdir, the double-folder packaging convention, so the game is inside.
#[test]
fn test_stray_file_next_to_a_like_named_wrapper_still_descends() {
    let temp_dir = tempfile_tempdir();
    let (game_root, inner) = wrapped_game(temp_dir.path());
    fs::write(
        game_root.join("OnlineFix.ini"),
        b"[crack left by an older build]",
    )
    .unwrap();

    let rel = Path::new("steam_appid.txt");
    let targets = resolve_file_targets(rel, &game_root, true);

    assert_eq!(targets, vec![inner.join("steam_appid.txt")]);
}

/// The folder name is not the only signal: a folder holding the game's own
/// executable is the game root even when the folder is named differently.
#[test]
fn test_wrapper_folder_holding_the_game_exe_descends_when_named_differently() {
    let temp_dir = tempfile_tempdir();
    let game_root = temp_dir.path().join(WRAPPER);
    let inner = game_root.join("ActualGame");
    fs::create_dir_all(&inner).unwrap();
    fs::write(inner.join("How to Fish.exe"), b"MZ fake").unwrap();
    fs::write(game_root.join("readme.txt"), b"repack notes").unwrap();

    let rel = Path::new("steam_appid.txt");
    let targets = resolve_file_targets(rel, &game_root, true);

    assert_eq!(targets, vec![inner.join("steam_appid.txt")]);
}

/// Guard: an executable at the installdir level means the game may live there,
/// so a folder named like the installdir that also holds the game's executable
/// must not steal the files.
#[test]
fn test_installdir_with_its_own_exe_never_descends() {
    let temp_dir = tempfile_tempdir();
    let (game_root, _inner) = wrapped_game(temp_dir.path());
    fs::write(game_root.join("How to Fish.exe"), b"MZ shipping exe").unwrap();

    let rel = Path::new("steam_appid.txt");
    let targets = resolve_file_targets(rel, &game_root, true);

    assert_eq!(targets, vec![game_root.join("steam_appid.txt")]);
}

/// Guard: without a signal that the folder holds the game - it is not named
/// like the installdir and holds no executable of the game - the base stays the
/// installdir. This is the residual gap, and no removal can close it, because
/// the folder itself carries no signal.
#[test]
fn test_folder_without_a_game_signal_keeps_the_installdir_as_base() {
    let temp_dir = tempfile_tempdir();
    let game_root = temp_dir.path().join(WRAPPER);
    fs::create_dir_all(game_root.join("Redist")).unwrap();
    fs::write(
        game_root.join("Redist").join("vc_redist.x64.exe"),
        b"MZ redist",
    )
    .unwrap();
    fs::write(
        game_root.join("OnlineFix.ini"),
        b"[crack left by an older build]",
    )
    .unwrap();

    let rel = Path::new("steam_appid.txt");
    let targets = resolve_file_targets(rel, &game_root, true);

    assert_eq!(targets, vec![game_root.join("steam_appid.txt")]);
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
