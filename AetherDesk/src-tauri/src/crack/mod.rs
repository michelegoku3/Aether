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
pub mod locate;

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
) -> Result<CrackReport, String> {
    if sources.is_empty() {
        return Err("No crack files selected.".to_string());
    }

    let staging = archive::create_staging(app_id)?;
    let mut report = CrackReport::default();

    // Ensure staging is cleaned up even when a source fails mid-way.
    let result = (|| -> Result<(), String> {
        for source in sources {
            report.sources += 1;
            let source_path = PathBuf::from(source.as_str());
            if !source_path.is_file() {
                return Err(format!("Crack file not found: {}", source_path.display()));
            }

            // Stage: extract (or copy, for loose files) into the staging root.
            archive::stage_source(&source_path, &staging, DEFAULT_ARCHIVE_PASSWORD)?;

            // Locate each staged file's destination inside the game and apply
            // (back up originals & crack files, write inventory, copy).
            let sub_report = apply::apply_staged_files(app_id, &staging, game_root, backup)?;
            report.applied += sub_report.applied;
            report.replaced += sub_report.replaced;
            report.files.extend(sub_report.files);

            // Clear staged files before processing the next source.
            archive::clear_staging_contents(&staging)?;
        }
        Ok(())
    })();

    // Best-effort cleanup regardless of success/error.
    let _ = archive::remove_staging(&staging);

    result.map_err(|error| error)?;
    Ok(report)
}
