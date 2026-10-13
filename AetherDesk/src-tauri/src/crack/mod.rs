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

/// Default password tried for password-protected archives.
const DEFAULT_ARCHIVE_PASSWORD: &str = "online-fix.me";

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
            // we stage it as an archive by copying it to a temporary `.exe.zip`
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
                let stage_res = archive::stage_source(&temp_zip, staging, DEFAULT_ARCHIVE_PASSWORD);
                let _ = std::fs::remove_file(&temp_zip);

                if stage_res.is_err() {
                    crate::desk_log_info!("crack", "Opening VN .exe patch as archive failed (not a self-extracting archive). Launching installer executable: {}", source_path.display());
                    // If opening the `.exe` as an archive fails (it is not a self-extracting
                    // archive), we do NOT copy the `.exe` into the game folder. Instead,
                    // as requested, we simply launch the executable patcher with current
                    // directory set to `game_root`.
                    #[cfg(target_os = "windows")]
                    {
                        let _ = std::process::Command::new(&source_path)
                            .current_dir(game_root)
                            .spawn();
                    }
                    report.applied += 1;
                    report.files.push(format!(
                        "(Launched patch installer: {})",
                        source_path
                            .file_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or_default()
                    ));
                    continue;
                }
            } else {
                archive::stage_source(&source_path, staging, DEFAULT_ARCHIVE_PASSWORD)?;
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
}
