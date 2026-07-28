use crate::settings::SettingsManager;
use crate::steam_library::{InstalledSteamGame, SteamLibraryScanner};
use crate::steamless::{
    SteamlessRunRequest, SteamlessRunResult, SteamlessRunner, SteamlessToolLocator,
};
use std::path::PathBuf;
use tauri_plugin_dialog::{DialogExt, FilePath};

const STEAMLESS_TIMEOUT_SECONDS: u64 = 120;

#[tauri::command]
pub async fn pick_and_run_steamless(
    app: tauri::AppHandle,
    app_id: u32,
) -> Result<SteamlessRunResult, String> {
    let game = resolve_installed_game(&app, app_id)?;
    let game_root = PathBuf::from(&game.game_path);

    let selected_file = app
        .dialog()
        .file()
        .set_title(format!("Select executable for {}", game.name))
        .set_directory(&game_root)
        .add_filter("Executables", &["exe"])
        .blocking_pick_file();

    let Some(file_path) = selected_file else {
        return Ok(SteamlessRunResult {
            success: false,
            cancelled: true,
            message: "Steamless cancelled.".to_string(),
            exe_path: None,
            backup_path: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
        });
    };

    let exe_path = file_path_to_path_buf(file_path)?;
    let tool = SteamlessToolLocator::new(app.clone()).locate()?;

    tauri::async_runtime::spawn_blocking(move || {
        SteamlessRunner::new(tool).run(SteamlessRunRequest {
            exe_path,
            game_root,
            timeout_seconds: STEAMLESS_TIMEOUT_SECONDS,
        })
    })
    .await
    .map_err(|e| format!("Steamless worker failed: {}", e))?
}

fn resolve_installed_game(
    app: &tauri::AppHandle,
    app_id: u32,
) -> Result<InstalledSteamGame, String> {
    let settings = SettingsManager::new(app).load();
    if settings.steam_path.trim().is_empty() {
        return Err("Steam installation path is required before using Steamless.".to_string());
    }

    let scanner = SteamLibraryScanner::new(settings.steam_path, Some(settings.active_library));
    let Some(game) = scanner
        .scan_installed_games()
        .into_iter()
        .find(|game| game.id == app_id)
    else {
        return Err(format!(
            "App ID {} was not found in the Lua library.",
            app_id
        ));
    };

    if !game.installed || game.game_path.trim().is_empty() {
        return Err("Steamless requires the selected game to be installed locally.".to_string());
    }

    let game_path = PathBuf::from(&game.game_path);
    if !game_path.is_dir() {
        return Err(format!(
            "The selected game's install folder was not found: {}",
            game_path.display()
        ));
    }

    Ok(game)
}

fn file_path_to_path_buf(file_path: FilePath) -> Result<PathBuf, String> {
    file_path.into_path().map_err(|e| {
        format!(
            "Selected executable path is not a local filesystem path: {}",
            e
        )
    })
}
