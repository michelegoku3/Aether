// Applying the located crack files to a game.
//
// Every staged file is individually resolved to its destination inside the
// game (see `locate::resolve_file_target`). For each one we:
//   1. back up the existing game file (if any) into `backup/original/`,
//   2. store a copy of the crack file into `backup/crack/`,
//   3. copy the crack file into the resolved destination,
//   4. record its game-relative path in an inventory file `crack_<app_id>.txt`
//      stored under `backup/original/`.
//
// The inventory is a plain, line-per-file list of every game-relative path the
// crack touched, so the operation is reversible and auditable.
use crate::core::backup::GameBackup;
use crate::crack::locate;
use std::fs;
use std::path::{Path, PathBuf};

/// Summary of one crack application run.
#[derive(Debug, Default)]
pub struct ApplyReport {
    pub applied: usize,
    pub replaced: usize,
    pub files: Vec<String>,
}

/// Apply every staged file under `staging` into `game_root`.
pub fn apply_staged_files(
    app_id: u32,
    staging: &Path,
    game_root: &Path,
    backup: &GameBackup,
) -> Result<ApplyReport, String> {
    let staged = collect_files(staging)?;
    if staged.is_empty() {
        return Err("No crack files found in the selected archive.".to_string());
    }

    let mut report = ApplyReport::default();
    let mut inventory: Vec<String> = Vec::new();
    let mut applied_targets: Vec<(PathBuf, PathBuf)> = Vec::new(); // (source, target)

    // Check if all staged files are normal files without folders (no subdirectory components).
    let is_flat_crack = staged.iter().all(|abs| {
        abs.strip_prefix(staging)
            .map(|rel| rel.components().count() == 1)
            .unwrap_or(false)
    });

    for abs in &staged {
        let archive_rel = abs.strip_prefix(staging).map_err(|_| {
            format!(
                "Internal error: file {} is outside the staging root {}",
                abs.display(),
                staging.display()
            )
        })?;

        // Where does this crack file actually belong inside the game?
        // When `is_flat_crack` is true, we recursively search for matching files
        // across all game subfolders and replace every match; if no matches exist,
        // it falls back to placing the file at the game root.
        let targets = locate::resolve_file_targets(archive_rel, game_root, is_flat_crack);

        for target in targets {
            let game_rel = target.strip_prefix(game_root).map_err(|_| {
                format!(
                    "Internal error: target {} is outside the game root {}",
                    target.display(),
                    game_root.display()
                )
            })?;
            let game_rel_string = game_rel.to_string_lossy().to_string();

            // 1. Back up the original game file if it will be replaced.
            if target.exists() {
                let backup_dest = backup.original_dir().join(&game_rel_string);
                copy_preserving(&target, backup_dest, "back up original")?;
                report.replaced += 1;
            }

            // 2. Store the crack file in the dedicated crack backup folder.
            let crack_dest = backup.crack_dir().join(&game_rel_string);
            copy_preserving(abs, crack_dest, "store crack file")?;

            // 3. Apply the crack to the resolved destination.
            copy_preserving(abs, &target, "apply crack")?;

            report.applied += 1;
            report.files.push(game_rel_string.clone());
            inventory.push(game_rel_string);
            applied_targets.push((abs.clone(), target));
        }
    }

    // Verify all files were actually copied to their destinations
    let mut missing_files = Vec::new();
    for (_source, target) in &applied_targets {
        if !target.exists() {
            missing_files.push(format!(
                "{}",
                target.strip_prefix(game_root).unwrap_or(target).display()
            ));
        }
    }

    if !missing_files.is_empty() {
        let file_list = missing_files
            .iter()
            .map(|f| format!("  • {}", f))
            .collect::<Vec<_>>()
            .join("\n");
        
        return Err(format!(
            "Crack application failed! {} file(s) were not applied:\n\n{}\n\n\
             This may be caused by Steam still processing the game files. \
             Restart Steam through Aether, and re-apply the crack.",
            missing_files.len(),
            file_list
        ));
    }

    write_inventory(backup, app_id, &inventory)?;
    Ok(report)
}

/// Recursively collect all files under `root`, sorted for stable output.
///
/// Every file in a crack archive is applied — including `.txt`, `.url`, docs,
/// images, etc. — because online-fix packs often place real payload in those
/// files (e.g. `dlllist.txt`, `OnlineFix.url`). There is deliberately no
/// "noise" filter.
fn collect_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    collect_files_into(root, &mut result)?;
    result.sort();
    Ok(result)
}

fn collect_files_into(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|error| format!("Failed to read folder {}: {}", dir.display(), error))?;

    for entry in entries {
        let entry = entry.map_err(|error| format!("Failed to read folder entry: {}", error))?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_into(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// Write the inventory file (game-relative paths only, one per line) under the
/// `original` backup folder.
fn write_inventory(backup: &GameBackup, app_id: u32, files: &[String]) -> Result<(), String> {
    let mut content = String::new();
    for file in files {
        content.push_str(file);
        content.push('\n');
    }

    let inventory_path = backup
        .original_dir()
        .join(format!("crack_{}.txt", app_id));
    fs::write(&inventory_path, content).map_err(|error| {
        format!(
            "Failed to write inventory {}: {}",
            inventory_path.display(),
            error
        )
    })
}

/// Copy `src` to `dst`, creating parent directories as needed.
fn copy_preserving<P, Q>(src: P, dst: Q, action: &str) -> Result<(), String>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    let src = src.as_ref();
    let dst = dst.as_ref();

    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!("Failed to create folder {}: {}", parent.display(), error)
        })?;
    }

    fs::copy(src, dst).map_err(|error| {
        format!("Failed to {} {} → {}: {}", action, src.display(), dst.display(), error)
    })?;
    Ok(())
}
