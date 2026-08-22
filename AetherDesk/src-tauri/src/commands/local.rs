// Local download commands (thin Tauri wrapper).
//
// All heavy lifting (archive extraction, folder copying, Steam folder
// resolution, backup) lives in the Tauri-agnostic engine `crate::local`.
// This file only opens the file picker, loads settings and calls the engine.
use crate::core::backup::GameBackup;
use crate::core::settings::SettingsManager;
use crate::local;
use crate::manifest::pins::LuaManifestPins;
use std::path::PathBuf;
use tauri_plugin_dialog::{DialogExt, FilePath};

/// Open the native file picker and return the chosen paths (possibly empty if
/// the user cancelled). Multiple files can be selected at once. The dialog
/// opens in the OS Downloads folder (where game archives are usually saved).
#[tauri::command]
pub async fn pick_local_files(
    app: tauri::AppHandle,
    _app_id: Option<u32>,
) -> Result<Vec<String>, String> {
    let start_dir = dirs::download_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let picker = app
        .dialog()
        .file()
        .set_title("Select game file(s), folder(s) or archive(s)")
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

/// Open the native folder picker and return the chosen directory path.
#[tauri::command]
pub async fn pick_local_folder(
    app: tauri::AppHandle,
) -> Result<Option<String>, String> {
    let start_dir = dirs::download_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let picker = app
        .dialog()
        .file()
        .set_title("Select folder containing Lua or Manifest files")
        .set_directory(&start_dir);

    let selected = picker.blocking_pick_folder();
    Ok(selected.and_then(file_path_to_string))
}

/// Bulk import any files, folders, and archives (.zip/.rar/.7z), recursively
/// discovering and routing every `.lua` and `.manifest` file into Steam.
#[tauri::command]
pub async fn install_bulk_local(
    app: tauri::AppHandle,
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

    crate::desk_log_info!(
        "local",
        "Bulk local import started with {} source item(s)",
        local_files.len()
    );

    let report = local::install_bulk_local_pipeline(
        &steam_path,
        &local_files,
        settings.download_games_with_updates_on,
    )?;

    crate::desk_log_info!(
        "local",
        "Bulk local import completed: {} .lua, {} .manifest across {} game(s)",
        report.lua_files,
        report.manifest_files,
        report.unique_apps
    );

    let msg = if report.unique_apps > 0 {
        format!(
            "Bulk import completed: {} Lua unlock file(s) and {} manifest file(s) installed into Steam across {} game(s).",
            report.lua_files, report.manifest_files, report.unique_apps
        )
    } else {
        format!(
            "Bulk import completed: {} Lua unlock file(s) and {} manifest file(s) installed into Steam.",
            report.lua_files, report.manifest_files
        )
    };

    Ok(msg)
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

    // Validate build-labelled loose Lua files concurrently. This is advisory:
    // providers may offer only the closest historical snapshot, which is still
    // useful, so mismatches become visible warnings and never reject a file.
    let token = crate::versioning::sources::resolve_build_details_token(Some(
        &settings.build_details_token,
    ))
    .unwrap_or_else(|| crate::versioning::sources::DEFAULT_BUILD_DETAILS_TOKEN.to_string());
    let mut validation_tasks = tokio::task::JoinSet::new();
    for file in &local_files {
        let path = PathBuf::from(file);
        let is_lua = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.eq_ignore_ascii_case("lua"))
            .unwrap_or(false);
        if is_lua {
            let task_token = token.clone();
            validation_tasks.spawn(async move {
                crate::versioning::lua_validation::validate_claimed_build(&path, task_token).await
            });
        }
    }
    let mut validation_warnings = Vec::new();
    while let Some(result) = validation_tasks.join_next().await {
        match result {
            Ok(Ok(Some(warning))) => {
                crate::desk_log_warn!("local", "{}", warning);
                validation_warnings.push(warning);
            }
            Ok(Ok(None)) => {}
            Ok(Err(error)) => crate::desk_log_warn!(
                "local",
                "Build validation unavailable; continuing without rejection: {}",
                error
            ),
            Err(error) => crate::desk_log_warn!(
                "local",
                "Build validation task failed; continuing without rejection: {}",
                error
            ),
        }
    }

    let report = local::install_local_pipeline(
        app_id,
        &app_name,
        &steam_path,
        active_library.as_deref(),
        &local_files,
    )?;

    crate::desk_log_info!("local", "Successfully installed local content for AppID {}: {} file(s) ({} lua, {} manifest), game files into {}",
        app_id, report.applied, report.lua_files, report.manifest_files, report.target);

    // Local packages follow the same update policy as every remote provider.
    // When updates are enabled, comment active setManifestid rows in the live
    // canonical Lua and refresh the canonical backup with the final bytes.
    if report.lua_files > 0 && settings.download_games_with_updates_on {
        let lua = LuaManifestPins::new(steam_path.clone(), app_id);
        lua.set_updates_enabled(true)?;
        let installed_lua = std::fs::read_to_string(lua.lua_path())
            .map_err(|error| format!("Failed to read the installed Lua after applying update policy: {error}"))?;
        GameBackup::for_app(app_id)?
            .backup_lua_artifacts(app_id, &installed_lua, &[])?;
    }

    let mut msg = format!(
        "Local install completed: {} file(s) installed ({} lua, {} manifest). Original sources backed up in AetherData/backup/{}.",
        report.applied,
        report.lua_files,
        report.manifest_files,
        app_id
    );
    if !validation_warnings.is_empty() {
        msg.push_str(" ");
        msg.push_str(&validation_warnings.join(" "));
    }

    Ok(msg)
}

fn file_path_to_string(file_path: FilePath) -> Option<String> {
    file_path
        .into_path()
        .ok()
        .map(|path| path.to_string_lossy().to_string())
}

/// List every archived Lua version for a game (current backup + history/).
/// Data source for the future "all builds" tab in Change Version.
#[tauri::command]
pub async fn list_lua_history(
    app_id: u32,
) -> Result<Vec<crate::core::backup::LuaHistoryEntry>, String> {
    crate::core::backup::list_lua_history(app_id)
}
