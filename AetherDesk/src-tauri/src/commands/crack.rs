// Apply Crack commands (thin Tauri wrapper).
//
// All heavy lifting (archive extraction, locating the real crack files, backup,
// inventory, applying) lives in the Tauri-agnostic engine `crate::crack`.
// This file only resolves the installed game, builds the per-game backup, and
// calls the engine.
use crate::core::backup::GameBackup;
use crate::util::game_resolver::resolve_installed_game;
use crate::crack;
use std::path::PathBuf;
use tauri_plugin_dialog::{DialogExt, FilePath};

/// Open the native file picker and return the chosen paths (possibly empty if
/// the user cancelled). Multiple files can be selected at once. The dialog
/// opens in the OS Downloads folder (where crack archives are usually saved).
#[tauri::command]
pub async fn pick_crack_files(
    app: tauri::AppHandle,
    _app_id: u32,
) -> Result<Vec<String>, String> {
    let start_dir = dirs::download_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let picker = app
        .dialog()
        .file()
        .set_title("Select crack file(s)")
        .set_directory(&start_dir);

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

/// Apply the selected crack sources to the installed game.
///
/// Each source may be a `.zip`, `.rar`, `.7z` or a loose crack file. The engine
/// extracts (trying the default password `online-fix.me` for protected
/// archives), resolves each file to its real destination in the game, backs up
/// originals & crack files, writes an inventory and applies the crack.
#[tauri::command]
pub async fn apply_crack(
    app: tauri::AppHandle,
    app_id: u32,
    crack_files: Vec<String>,
    vn_patch_mode: Option<bool>,
) -> Result<String, String> {
    let game = resolve_installed_game(&app, app_id)?;
    let game_root = PathBuf::from(&game.game_path);
    let backup = GameBackup::for_app(app_id)?;

    let vn_patch_mode = vn_patch_mode.unwrap_or(false);
    let report =
        crack::apply_crack_pipeline(app_id, &game_root, &backup, &crack_files, vn_patch_mode)?;

    // Show the full game-relative path of each applied file.
    let names: Vec<String> = report.files.clone();

    let mut msg = format!(
        "Crack applied: {} file(s) ({} replaced). Originals & crack files backed up. Files: {}",
        report.applied,
        report.replaced,
        names.join(", ")
    );

    if msg.len() > 500 {
        let mut cutoff = 497;
        while !msg.is_char_boundary(cutoff) && cutoff > 0 {
            cutoff -= 1;
        }
        msg.truncate(cutoff);
        msg.push_str("...");
    }

    Ok(msg)
}

fn file_path_to_string(file_path: FilePath) -> Option<String> {
    file_path
        .into_path()
        .ok()
        .map(|path| path.to_string_lossy().to_string())
}
