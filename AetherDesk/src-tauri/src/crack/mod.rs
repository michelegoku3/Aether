// Crack application engine.
//
// Owns the full "apply crack" pipeline for one game:
//   extract each source (zip/rar/7z/plain) → resolve each file's destination
//   in the game → back up originals & crack files → write an inventory →
//   copy into the game.
//
// This module is deliberately Tauri- and Steam-agnostic: it only needs the
// game's install root, the per-game `GameBackup`, and the list of source paths.
// The thin Tauri command in `commands/crack.rs` is just a wrapper around it.
pub mod apply;
pub mod archive;
pub mod hashing;
pub mod io;
pub mod locate;
pub mod paths;
pub mod recovery;

use crate::core::backup::GameBackup;
use std::path::{Path, PathBuf};
#[cfg(target_os = "windows")]
use std::process::{Child, ExitStatus};
#[cfg(target_os = "windows")]
use std::thread;
#[cfg(target_os = "windows")]
use std::time::{Duration, Instant};

/// Human-readable summary of one crack application run.
#[derive(Debug, Default)]
pub struct CrackReport {
    /// Number of source files processed (archives or loose files).
    pub sources: usize,
    /// Number of files applied to the game.
    pub applied: usize,
    /// Number of existing game files that were backed up (replaced).
    pub replaced: usize,
    /// Relative paths of all applied files.
    pub files: Vec<String>,
    /// Sources that produced no applied file, in the order they were tried.
    ///
    /// Kept apart from `files` because a failure is not an applied path: the
    /// command layer turns a run with only failures into an error instead of a
    /// success message listing nothing.
    pub failures: Vec<String>,
}

/// Launch the VN patch installer with `game_root` as its working directory and
/// report how it ended.
///
/// The caller counts the patch as applied only on `Ok(())`, so a failure to
/// start the installer or a non-zero exit code stays an error instead of being
/// swallowed. The wait is bounded: a graphical installer stays alive until the
/// user closes it, and the pipeline holds the per-game mutation lock while it
/// waits.
pub(crate) fn launch_patch_installer(source: &Path, game_root: &Path) -> Result<(), String> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (source, game_root);
        Err("unsupported platform".to_string())
    }
    #[cfg(target_os = "windows")]
    {
        let mut child = std::process::Command::new(source)
            .current_dir(game_root)
            .spawn()
            .map_err(|e| format!("failed to launch {}: {e}", source.display()))?;
        let status = poll_child_exit(&mut child, PATCH_INSTALLER_BUDGET)?;
        installer_exit_result(status)
    }
}

/// Maximum time a VN patch installer may run before the pipeline gives up.
#[cfg(target_os = "windows")]
const PATCH_INSTALLER_BUDGET: Duration = Duration::from_secs(180);

/// Interval between two liveness probes of the installer process.
#[cfg(target_os = "windows")]
const PATCH_INSTALLER_POLL: Duration = Duration::from_millis(50);

/// Waits for `child` to exit, polling until `budget` elapses.
///
/// The child keeps running when the budget runs out: the user may still be
/// working in the installer window, and killing it could leave the game half
/// patched.
#[cfg(target_os = "windows")]
pub(crate) fn poll_child_exit(child: &mut Child, budget: Duration) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + budget;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(error) => return Err(format!("failed to wait for the patch installer: {error}")),
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(format!("patch installer did not exit within {budget:?}"));
        }
        thread::sleep(PATCH_INSTALLER_POLL.min(deadline - now));
    }
}

/// Translate an installer's process status into the pipeline outcome.
#[cfg(target_os = "windows")]
pub(crate) fn installer_exit_result(status: std::process::ExitStatus) -> Result<(), String> {
    if status.success() {
        Ok(())
    } else {
        Err(format!("patch installer exited with {status}"))
    }
}

/// Run the crack pipeline for the given sources against `game_root`.
pub fn apply_crack_pipeline(
    app_id: u32,
    game_root: &Path,
    backup: &GameBackup,
    sources: &[String],
    vn_patch_mode: bool,
) -> Result<CrackReport, String> {
    if sources.is_empty() {
        return Err("No crack files selected.".to_string());
    }

    let staging = archive::create_staging(app_id)?;
    apply_sources_in_staging(app_id, game_root, backup, sources, vn_patch_mode, &staging)
}

// Explicit staging keeps regression tests entirely inside their TempDir.
fn apply_sources_in_staging(
    app_id: u32,
    game_root: &Path,
    backup: &GameBackup,
    sources: &[String],
    vn_patch_mode: bool,
    staging: &Path,
) -> Result<CrackReport, String> {
    let mut report = CrackReport::default();

    // Ensure staging is cleaned up even when a source fails mid-way.
    let result = (|| -> Result<(), String> {
        for source in sources {
            report.sources += 1;
            let source_path = PathBuf::from(source.as_str());
            if !source_path.is_file() {
                return Err(format!("Crack file not found: {}", source_path.display()));
            }

            // When `vn_patch_mode` is ON and the source file is an executable (.exe),
            // it is staged as an archive by copying it to a temporary `.exe.zip`
            // file in the staging directory's parent folder, so archive extractors
            // (zip / 7z / rar) open and extract its contents automatically.
            let is_vn_exe = vn_patch_mode
                && source_path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("exe"))
                    .unwrap_or(false);

            if is_vn_exe {
                let temp_zip = staging.parent().unwrap_or(staging).join(format!(
                    "temp_vn_{}.exe.zip",
                    source_path
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default()
                ));
                std::fs::copy(&source_path, &temp_zip)
                    .map_err(|e| format!("Failed to stage VN patch .exe as .exe.zip: {e}"))?;
                let stage_res =
                    archive::stage_source(&temp_zip, staging, archive::DEFAULT_ARCHIVE_PASSWORDS);
                let _ = std::fs::remove_file(&temp_zip);

                if stage_res.is_err() {
                    crate::desk_log_info!("crack", "Opening VN .exe patch as archive failed (not a self-extracting archive). Launching installer executable: {}", source_path.display());
                    // If opening the `.exe` as an archive fails (it is not a self-extracting
                    // archive), the `.exe` is NOT copied into the game folder. Instead the
                    // executable patcher is launched with the current directory set to
                    // `game_root`. Only a successful exit counts as applied; a failed
                    // launch is reported and the remaining sources still run.
                    let name = source_path
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default();
                    match launch_patch_installer(&source_path, game_root) {
                        Ok(()) => {
                            report.applied += 1;
                            report
                                .files
                                .push(format!("(Launched patch installer: {name})"));
                        }
                        Err(err) => {
                            crate::desk_log_error!(
                                "crack",
                                "Launching VN patch installer {} failed: {}",
                                source_path.display(),
                                err
                            );
                            report
                                .failures
                                .push(format!("Patch installer failed: {name}: {err}"));
                        }
                    }
                    continue;
                }
            } else {
                archive::stage_source(&source_path, staging, archive::DEFAULT_ARCHIVE_PASSWORDS)?;
            }

            // Locate each staged file's destination inside the game and apply
            // (back up originals & crack files, write inventory, copy).
            let sub_report = apply::apply_staged_files(app_id, staging, game_root, backup)?;
            report.applied += sub_report.applied;
            report.replaced += sub_report.replaced;
            report.files.extend(sub_report.files);

            // Clear staged files before processing the next source.
            archive::clear_staging_contents(staging)?;
        }
        Ok(())
    })();

    // Best-effort cleanup regardless of success/error.
    let _ = archive::remove_staging(staging);

    result?;
    Ok(report)
}

/// Probe whether a previously applied crack is stored under `backup/<app_id>/crack/`.
/// Does not create folders — pure read of existing backup state.
pub fn has_saved_crack(app_id: u32) -> bool {
    GameBackup::open_existing(app_id)
        .map(|backup| backup.has_saved_crack())
        .unwrap_or(false)
}

/// Re-apply every file stored in `backup/<app_id>/crack/` into the game install.
///
/// Layout under `crack/` mirrors game-relative paths, so no archive extraction
/// or locate heuristics are needed — each file is copied straight to its path.
/// A crack saved before the wrapped-game-root rule existed therefore keeps the
/// location it was saved with; removing it (which clears the recorded paths)
/// and applying the crack again is what moves those files next to the game.
pub fn reapply_saved_crack(
    app_id: u32,
    game_root: &Path,
    backup: &GameBackup,
) -> Result<CrackReport, String> {
    apply::reapply_saved_crack_files(app_id, game_root, backup).map(|sub| CrackReport {
        sources: 1,
        applied: sub.applied,
        replaced: sub.replaced,
        files: sub.files,
        // A re-apply copies stored files straight to their paths, so it has no
        // source that can fail without applying anything.
        failures: Vec::new(),
    })
}

/// Delete every game file listed in the crack inventory (the crack currently
/// present in the game install). Originals under `backup/original/` are left
/// untouched so the user can still recover them; the inventory itself is cleared.
pub fn remove_applied_crack_from_game(
    app_id: u32,
    game_root: &Path,
    backup: &GameBackup,
) -> Result<usize, String> {
    apply::remove_applied_crack_files(app_id, game_root, backup)
}

/// Error shown when a crack is applied while UCOnline2 is active: the two
/// Spacewar stacks overwrite each other, so UCO2 must be turned off first.
pub const UCO2_ACTIVE_ERROR: &str =
    "UCOnline2 is active in this game folder. Disable UCOnline2 first, apply the crack, then \
     enable UCOnline2 again only if the crack is not online-fix.me.";

/// True when UCOnline2 files are present in the game folder.
pub fn has_active_uco2(game_root: &Path) -> bool {
    crate::online::foreign::scan(game_root).uco2
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    const APP_ID: u32 = 424243;

    fn run(
        sources: &[(&str, &[u8])],
        fail_last: bool,
    ) -> (
        tempfile::TempDir,
        GameBackup,
        PathBuf,
        Result<CrackReport, String>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let backup_root = temp.path().join("backup");
        for directory in ["lua", "original", "crack"] {
            fs::create_dir_all(backup_root.join(directory)).unwrap();
        }
        let backup = GameBackup::for_tests(backup_root);
        let game = temp.path().join("game");
        let staging = temp.path().join("stage");
        let input = temp.path().join("input");
        fs::create_dir_all(&game).unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir_all(&input).unwrap();
        fs::write(game.join("a.dll"), b"original a").unwrap();
        let mut paths = Vec::new();
        for (index, (name, bytes)) in sources.iter().enumerate() {
            let directory = input.join(index.to_string());
            fs::create_dir_all(&directory).unwrap();
            let path = directory.join(name);
            fs::write(&path, bytes).unwrap();
            paths.push(path.to_str().unwrap().to_string());
        }
        if fail_last {
            paths.push(input.join("missing.dll").to_str().unwrap().to_string());
        }
        let result = apply_sources_in_staging(APP_ID, &game, &backup, &paths, false, &staging);
        assert!(
            !staging.exists(),
            "only explicit temporary staging is cleaned"
        );
        (temp, backup, game, result)
    }

    #[test]
    fn multiple_disjoint_sources_are_all_in_the_removal_inventory() {
        let (_temp, backup, game, result) =
            run(&[("a.dll", b"patch a"), ("b.dll", b"patch b")], false);
        let report = result.unwrap();
        assert_eq!(report.sources, 2);
        assert_eq!(report.applied, 2);
        assert_eq!(
            backup.read_crack_inventory(APP_ID).unwrap(),
            vec!["a.dll", "b.dll"]
        );
        assert_eq!(
            remove_applied_crack_from_game(APP_ID, &game, &backup).unwrap(),
            2
        );
        assert_eq!(
            std::fs::read(backup.original_dir().join("a.dll")).unwrap(),
            b"original a"
        );
        assert!(!backup.original_dir().join("b.dll").exists());
    }

    #[test]
    fn overlapping_sources_keep_original_and_apply_the_last_payload() {
        let (_temp, backup, game, result) =
            run(&[("a.dll", b"patch one"), ("a.dll", b"patch two")], false);
        assert_eq!(result.unwrap().applied, 2);
        assert_eq!(fs::read(game.join("a.dll")).unwrap(), b"patch two");
        assert_eq!(
            fs::read(backup.original_dir().join("a.dll")).unwrap(),
            b"original a"
        );
        assert_eq!(backup.read_crack_inventory(APP_ID).unwrap(), vec!["a.dll"]);
    }

    #[test]
    fn later_source_failure_leaves_a_complete_recoverable_prefix() {
        let (_temp, backup, game, result) = run(&[("a.dll", b"patch a")], true);
        assert!(result.is_err());
        assert_eq!(fs::read(game.join("a.dll")).unwrap(), b"patch a");
        assert_eq!(backup.read_crack_inventory(APP_ID).unwrap(), vec!["a.dll"]);
        assert_eq!(
            remove_applied_crack_from_game(APP_ID, &game, &backup).unwrap(),
            1
        );
        assert_eq!(
            fs::read(backup.original_dir().join("a.dll")).unwrap(),
            b"original a"
        );
    }

    #[test]
    fn uco2_files_are_reported_as_active() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("game");
        fs::create_dir_all(&game).unwrap();

        assert!(!has_active_uco2(&game));

        fs::write(game.join("union-crax.ini"), b"[Settings]").unwrap();
        assert!(has_active_uco2(&game));
    }
}
