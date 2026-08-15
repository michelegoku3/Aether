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
    crate::desk_log_info!("crack", "Applying crack for AppID {} with {} source file(s) (vn_patch_mode: {})",
        app_id, crack_files.len(), vn_patch_mode);

    let report =
        crack::apply_crack_pipeline(app_id, &game_root, &backup, &crack_files, vn_patch_mode)?;

    crate::desk_log_info!("crack", "Successfully applied crack for AppID {}: {} files applied, {} replaced",
        app_id, report.applied, report.replaced);

    Ok(format_apply_message(
        "Crack applied",
        report.applied,
        report.replaced,
        &report.files,
    ))
}

/// Whether `AetherData/backup/<app_id>/crack/` already holds saved crack files.
#[tauri::command]
pub fn has_saved_crack(app_id: u32) -> bool {
    crack::has_saved_crack(app_id)
}

/// Re-apply the crack previously stored under `backup/<app_id>/crack/`.
#[tauri::command]
pub async fn reapply_saved_crack(app: tauri::AppHandle, app_id: u32) -> Result<String, String> {
    let game = resolve_installed_game(&app, app_id)?;
    let game_root = PathBuf::from(&game.game_path);
    let backup = GameBackup::for_app(app_id)?;

    if !backup.has_saved_crack() {
        return Err("No saved crack found for this game.".to_string());
    }

    crate::desk_log_info!(
        "crack",
        "Re-applying saved crack for AppID {} from {}",
        app_id,
        backup.crack_dir().display()
    );

    let report = crack::reapply_saved_crack(app_id, &game_root, &backup)?;

    crate::desk_log_info!(
        "crack",
        "Re-applied saved crack for AppID {}: {} files ({} replaced)",
        app_id,
        report.applied,
        report.replaced
    );

    Ok(format_apply_message(
        "Saved crack re-applied",
        report.applied,
        report.replaced,
        &report.files,
    ))
}

/// Remove the crack currently present in the game install (inventory / saved
/// crack paths). Used when the user declines reusing the saved crack and wants
/// a fresh drop/apply flow instead.
#[tauri::command]
pub async fn remove_applied_crack(app: tauri::AppHandle, app_id: u32) -> Result<String, String> {
    let game = resolve_installed_game(&app, app_id)?;
    let game_root = PathBuf::from(&game.game_path);
    let backup = GameBackup::for_app(app_id)?;

    crate::desk_log_info!(
        "crack",
        "Removing applied crack from game for AppID {}",
        app_id
    );

    let removed = crack::remove_applied_crack_from_game(app_id, &game_root, &backup)?;

    crate::desk_log_info!(
        "crack",
        "Removed {} applied crack file(s) for AppID {}",
        removed,
        app_id
    );

    if removed == 0 {
        Ok("No applied crack files were found in the game folder.".to_string())
    } else {
        Ok(format!(
            "Removed {} crack file(s) from the game. You can drop a new crack now.",
            removed
        ))
    }
}

fn format_apply_message(prefix: &str, applied: usize, replaced: usize, files: &[String]) -> String {
    let mut msg = format!(
        "{prefix}: {applied} file(s) ({replaced} replaced). Files: {}",
        files.join(", ")
    );
    if msg.len() > 500 {
        let mut cutoff = 497;
        while !msg.is_char_boundary(cutoff) && cutoff > 0 {
            cutoff -= 1;
        }
        msg.truncate(cutoff);
        msg.push_str("...");
    }
    msg
}

fn file_path_to_string(file_path: FilePath) -> Option<String> {
    file_path
        .into_path()
        .ok()
        .map(|path| path.to_string_lossy().to_string())
}
