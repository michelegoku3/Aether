// Apply Crack commands (thin Tauri wrapper).
//
// All heavy lifting (archive extraction, locating the real crack files, backup,
// inventory, applying) lives in the Tauri-agnostic engine `crate::crack`.
// This file only resolves the installed game, builds the per-game backup, and
// calls the engine.
use crate::core::backup::GameBackup;
use crate::crack;
use crate::util::dialog::file_path_to_string;
use crate::util::game_resolver::resolve_installed_game;
use std::path::PathBuf;
use tauri_plugin_dialog::DialogExt;

/// Open the native file picker and return the chosen paths (possibly empty if
/// the user cancelled). Multiple files can be selected at once. The dialog
/// opens in the OS Downloads folder (where crack archives are usually saved).
#[tauri::command]
pub async fn pick_crack_files(app: tauri::AppHandle, _app_id: u32) -> Result<Vec<String>, String> {
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
    let mut paths = Vec::new();
    for file_path in selected.into_iter().flatten() {
        if let Some(path) = file_path_to_string(file_path)? {
            paths.push(path);
        }
    }
    Ok(paths)
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
    let steam_root = crate::commands::command_steam_path(&app)?;
    let _mutation = crate::core::game_mutations::acquire(
        std::path::Path::new(&steam_root),
        app_id,
        "apply_crack",
    )
    .await?;
    crate::core::game_mutations::ensure_current_root(&app, std::path::Path::new(&steam_root))?;
    let game = resolve_installed_game(&app, app_id)?;
    let game_root = PathBuf::from(&game.game_path);
    if crack::has_active_uco2(&game_root) {
        return Err(crack::UCO2_ACTIVE_ERROR.to_string());
    }
    let backup = GameBackup::for_app(app_id)?;

    let vn_patch_mode = vn_patch_mode.unwrap_or(false);
    crate::desk_log_info!(
        "crack",
        "Applying crack for AppID {} with {} source file(s) (vn_patch_mode: {})",
        app_id,
        crack_files.len(),
        vn_patch_mode
    );

    let report =
        crack::apply_crack_pipeline(app_id, &game_root, &backup, &crack_files, vn_patch_mode)?;

    crate::desk_log_info!(
        "crack",
        "Successfully applied crack for AppID {}: {} files applied, {} replaced",
        app_id,
        report.applied,
        report.replaced
    );

    apply_outcome(
        "Crack applied",
        report.applied,
        report.replaced,
        &report.files,
        &report.failures,
    )
}

/// Whether `AetherData/backup/<app_id>/crack/` already holds saved crack files.
#[tauri::command]
pub fn has_saved_crack(app_id: u32) -> bool {
    crack::has_saved_crack(app_id)
}

/// Re-apply the crack previously stored under `backup/<app_id>/crack/`.
#[tauri::command]
pub async fn reapply_saved_crack(app: tauri::AppHandle, app_id: u32) -> Result<String, String> {
    let steam_root = crate::commands::command_steam_path(&app)?;
    let _mutation = crate::core::game_mutations::acquire(
        std::path::Path::new(&steam_root),
        app_id,
        "reapply_saved_crack",
    )
    .await?;
    crate::core::game_mutations::ensure_current_root(&app, std::path::Path::new(&steam_root))?;
    let game = resolve_installed_game(&app, app_id)?;
    let game_root = PathBuf::from(&game.game_path);
    if crack::has_active_uco2(&game_root) {
        return Err(crack::UCO2_ACTIVE_ERROR.to_string());
    }
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

    apply_outcome(
        "Saved crack re-applied",
        report.applied,
        report.replaced,
        &report.files,
        &report.failures,
    )
}

/// Remove the crack currently present in the game install (inventory / saved
/// crack paths). Used when the user declines reusing the saved crack and wants
/// a fresh drop/apply flow instead.
#[tauri::command]
pub async fn remove_applied_crack(app: tauri::AppHandle, app_id: u32) -> Result<String, String> {
    let steam_root = crate::commands::command_steam_path(&app)?;
    let _mutation = crate::core::game_mutations::acquire(
        std::path::Path::new(&steam_root),
        app_id,
        "remove_applied_crack",
    )
    .await?;
    crate::core::game_mutations::ensure_current_root(&app, std::path::Path::new(&steam_root))?;
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

/// Turn one crack run into the value the UI shows, or into an error.
///
/// A run that applied nothing is an error even when the engine itself
/// returned `Ok`: the VN patch installer reports its launch failure through
/// the report instead of aborting the pipeline, and presenting that as a
/// success would paint a failed crack green. A run that applied some files
/// keeps the success message and appends the failures so nothing is lost.
fn apply_outcome(
    prefix: &str,
    applied: usize,
    replaced: usize,
    files: &[String],
    failures: &[String],
) -> Result<String, String> {
    if applied == 0 {
        if !failures.is_empty() {
            return Err(cap_message(failures.join("; ")));
        }
        if files.is_empty() {
            return Err("No files were applied.".to_string());
        }
    }

    let mut msg = format!(
        "{prefix}: {applied} file(s) ({replaced} replaced). Files: {}",
        files.join(", ")
    );
    if !failures.is_empty() {
        msg.push_str(" Failed: ");
        msg.push_str(&failures.join("; "));
    }
    Ok(cap_message(msg))
}

/// Bound the message length: a crack with hundreds of files must not produce a
/// payload the UI has to render in full.
fn cap_message(mut msg: String) -> String {
    const MAX: usize = 500;
    if msg.len() > MAX {
        let mut cutoff = MAX - 3;
        while !msg.is_char_boundary(cutoff) && cutoff > 0 {
            cutoff -= 1;
        }
        msg.truncate(cutoff);
        msg.push_str("...");
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_failures_is_an_error_with_just_the_failure_text() {
        let files: Vec<String> = Vec::new();
        let failures =
            vec!["Patch installer failed: ciao.exe: failed to launch ciao.exe".to_string()];

        let outcome = apply_outcome("Crack applied", 0, 0, &files, &failures);

        assert_eq!(
            outcome,
            Err("Patch installer failed: ciao.exe: failed to launch ciao.exe".to_string())
        );
        assert!(!outcome.unwrap_err().contains("Crack applied"));
    }

    #[test]
    fn partial_success_keeps_the_message_and_appends_the_failure() {
        let files = vec!["a.dll".to_string(), "b.dll".to_string()];
        let failures = vec!["Patch installer failed: ciao.exe: boom".to_string()];

        let outcome = apply_outcome("Crack applied", 2, 1, &files, &failures).unwrap();

        assert!(outcome.starts_with("Crack applied: 2 file(s) (1 replaced). Files: a.dll, b.dll"));
        assert!(outcome.contains("Failed: Patch installer failed: ciao.exe: boom"));
    }

    #[test]
    fn nothing_applied_at_all_is_an_error() {
        let outcome = apply_outcome("Crack applied", 0, 0, &[], &[]);

        assert_eq!(outcome, Err("No files were applied.".to_string()));
    }
}
