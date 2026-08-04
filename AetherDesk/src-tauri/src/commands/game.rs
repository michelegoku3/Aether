// Shared game-resolution helper for Tauri commands.
//
// Several commands (Steamless, Apply Crack) need to turn an App ID into the
// on-disk folder of an installed Lua game. Keeping that logic here (in the
// command layer, where SettingsManager/AppHandle live) avoids duplicating it
// in every command while keeping the domain scanner (`steam_library`) free of
// Tauri dependencies.
use crate::settings::SettingsManager;
use crate::steam_library::{InstalledSteamGame, SteamLibraryScanner};
use std::path::PathBuf;

/// Resolve an installed Lua game from the configured Steam installation.
///
/// Returns an error when Steam is not configured, the App ID is not in the
/// Lua library, or the game is not installed locally.
pub fn resolve_installed_game(
    app: &tauri::AppHandle,
    app_id: u32,
) -> Result<InstalledSteamGame, String> {
    let settings = SettingsManager::new(app).load();
    if settings.steam_path.trim().is_empty() {
        return Err("Steam installation path is required.".to_string());
    }

    let scanner = SteamLibraryScanner::new(settings.steam_path, Some(settings.active_library));
    let Some(game) = scanner
        .scan_installed_games()
        .into_iter()
        .find(|game| game.id == app_id)
    else {
        return Err(format!("App ID {} was not found in the Lua library.", app_id));
    };

    if !game.installed || game.game_path.trim().is_empty() {
        return Err("The selected game must be installed locally.".to_string());
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
