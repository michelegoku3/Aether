// Apply Crack commands.
//
// This module owns the "Apply Crack" popup backend: picking one or more crack
// files via the native OS dialog, and copying them into the installed game's
// folder (with a backup of any file that gets overwritten). The frontend stays
// thin and only passes the App ID and the chosen file paths through `invoke()`.
use crate::commands::game::resolve_installed_game;
use std::fs;
use std::path::PathBuf;
use tauri_plugin_dialog::{DialogExt, FilePath};

/// Open the native file picker and return the chosen paths (possibly empty if
/// the user cancelled). Multiple files can be selected at once. The dialog
/// starts in the game's install folder when the game is installed, otherwise
/// in the OS default location.
#[tauri::command]
pub async fn pick_crack_files(
    app: tauri::AppHandle,
    app_id: u32,
) -> Result<Vec<String>, String> {
    let start_dir = resolve_installed_game(&app, app_id)
        .ok()
        .map(|game| PathBuf::from(&game.game_path));

    let mut picker = app
        .dialog()
        .file()
        .set_title("Select crack file(s)");
    if let Some(dir) = start_dir {
        picker = picker.set_directory(&dir);
    }

    let selected = picker.blocking_pick_files();
    // `blocking_pick_files()` returns `Option<Vec<FilePath>>`. `.into_iter()`
    // over the Option yields the inner `Vec<FilePath>` (0 or 1 items), so we
    // must `.flatten()` it before mapping each `FilePath` to a string.
    Ok(selected
        .into_iter()
        .flatten()
        .filter_map(file_path_to_string)
        .collect())
}

/// Copy the selected crack files into the installed game's folder.
///
/// If a file with the same name already exists there, it is backed up as
/// `<name>.bak` before being overwritten, so the operation is reversible.
#[tauri::command]
pub async fn apply_crack(
    app: tauri::AppHandle,
    app_id: u32,
    crack_files: Vec<String>,
) -> Result<String, String> {
    if crack_files.is_empty() {
        return Err("No crack files selected.".to_string());
    }

    let game = resolve_installed_game(&app, app_id)?;
    let game_dir = PathBuf::from(&game.game_path);

    let mut applied = Vec::new();
    let mut backups = 0usize;

    for crack_file in &crack_files {
        let src = PathBuf::from(crack_file);
        if !src.is_file() {
            return Err(format!("Crack file not found: {}", src.display()));
        }
        let Some(file_name) = src.file_name().map(|name| name.to_string_lossy().to_string())
        else {
            return Err(format!("Crack file has no valid name: {}", src.display()));
        };

        let dest = game_dir.join(&file_name);
        if dest.exists() {
            let backup = PathBuf::from(format!("{}.bak", dest.to_string_lossy()));
            fs::copy(&dest, &backup).map_err(|error| {
                format!(
                    "Failed to back up existing file {}: {}",
                    dest.display(),
                    error
                )
            })?;
            backups += 1;
        }

        fs::copy(&src, &dest).map_err(|error| {
            format!("Failed to copy crack to {}: {}", dest.display(), error)
        })?;
        applied.push(file_name);
    }

    Ok(format!(
        "Crack applied to {} file(s): {}{}.",
        applied.len(),
        applied.join(", "),
        if backups > 0 {
            format!(" (backed up {} existing file(s))", backups)
        } else {
            String::new()
        }
    ))
}

fn file_path_to_string(file_path: FilePath) -> Option<String> {
    file_path
        .into_path()
        .ok()
        .map(|path| path.to_string_lossy().to_string())
}
