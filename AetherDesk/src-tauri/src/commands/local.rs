// Local download commands (thin Tauri wrapper).
//
// All heavy lifting (archive extraction, folder copying, Steam folder
// resolution, backup) lives in the Tauri-agnostic engine `crate::local`.
// This file only opens the file picker, loads settings and calls the engine.
use crate::core::settings::SettingsManager;
use crate::local;
use std::path::PathBuf;
use tauri_plugin_dialog::{DialogExt, FilePath};

/// Open the native file picker and return the chosen paths (possibly empty if
/// the user cancelled). Multiple files can be selected at once. The dialog
/// opens in the OS Downloads folder (where game archives are usually saved).
#[tauri::command]
pub async fn pick_local_files(
    app: tauri::AppHandle,
    _app_id: u32,
) -> Result<Vec<String>, String> {
    let start_dir = dirs::download_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let picker = app
        .dialog()
        .file()
        .set_title("Select game file(s) or archive(s)")
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

/// Install the selected local sources into the game's Steam folder.
///
/// Each source may be a `.zip`, `.rar`, `.7z` (or any other archive readable
/// by the shared staging helper), a loose file or a dropped folder. The engine
/// extracts/copies everything into the game's Steam folder
/// (`steamapps/common/<game>`) and backs up the original sources into
/// `AetherData/backup/<app_id>/local`.
///
/// Unlike Apply Crack, the game does NOT need to be installed already: when no
/// appmanifest exists, the folder is created in the active library.
#[tauri::command]
pub async fn install_local_game(
    app: tauri::AppHandle,
    app_id: u32,
    app_name: String,
    local_files: Vec<String>,
) -> Result<String, String> {
    let settings = SettingsManager::new(&app).load();
    if settings.steam_path.trim().is_empty() {
        return Err("Steam installation path is required. Set it in Settings first.".to_string());
    }
    let steam_path = PathBuf::from(settings.steam_path.trim());
    if !steam_path.is_dir() {
        return Err(format!(
            "The configured Steam path was not found: {}",
            steam_path.display()
        ));
    }
    let active_library = {
        let value = settings.active_library.trim().to_string();
        if value.is_empty() { None } else { Some(value) }
    };

    crate::desk_log_info!("local", "Local install for AppID {} ({}) with {} source file(s)",
        app_id, app_name, local_files.len());

    let report = local::install_local_pipeline(
        app_id,
        &app_name,
        &steam_path,
        active_library.as_deref(),
        &local_files,
    )?;

    crate::desk_log_info!("local", "Successfully installed local content for AppID {}: {} file(s) ({} lua, {} manifest), game files into {}",
        app_id, report.applied, report.lua_files, report.manifest_files, report.target);

    let mut msg = format!(
        "Local install completed: {} file(s) installed ({} lua → config/stplug-in, {} manifest → depotcache, rest → {}). Original sources backed up in AetherData. Files: {}",
        report.applied,
        report.lua_files,
        report.manifest_files,
        report.target,
        report.files.join(", ")
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
