use super::command_steam_path;
use crate::achievements::store::{
    commit_achievement_changes_blocking, default_steam_gate, inspect_game_achievements,
    now_unix_seconds, AchievementChange, GameAchievementsSnapshot, SaveAchievementsReport,
};
use crate::core::paths::LocalAppPaths;
use std::path::PathBuf;

/// Inspects a game's local achievement schema, account list, and reconciled
/// unlock state across Steam's cache and Aether's backup mirrors.
#[tauri::command]
pub async fn get_game_achievements(
    app: tauri::AppHandle,
    app_id: u32,
    account_id: Option<u32>,
) -> Result<GameAchievementsSnapshot, String> {
    if app_id == 0 {
        return Err("A valid AppID is required.".to_string());
    }
    let steam_root = PathBuf::from(command_steam_path(&app)?);
    let data_root = LocalAppPaths::data_root();
    crate::desk_log_debug!(
        "achievements",
        "IPC get_game_achievements app_id={} requested_account={:?}",
        app_id,
        account_id
    );
    tauri::async_runtime::spawn_blocking(move || {
        inspect_game_achievements(&steam_root, &data_root, app_id, account_id)
    })
    .await
    .map_err(|err| format!("Achievement inspection task failed: {err}"))?
}

/// Applies explicit unlock/relock edits atomically across backup JSON, backup
/// `.bin`, and Steam's `appcache/stats` `.bin` under the per-game mutation lock.
#[tauri::command]
pub async fn save_game_achievements(
    app: tauri::AppHandle,
    app_id: u32,
    account_id: u32,
    edits: Vec<AchievementChange>,
    allow_steam_kill: bool,
) -> Result<SaveAchievementsReport, String> {
    if app_id == 0 || account_id == 0 {
        return Err("Valid AppID and account ID are required.".to_string());
    }
    let steam_path = command_steam_path(&app)?;
    let steam_root = PathBuf::from(&steam_path);
    let data_root = LocalAppPaths::data_root();
    crate::desk_log_info!(
        "achievements",
        "IPC save_game_achievements app_id={} account_id={} edits={} allow_steam_kill={}",
        app_id,
        account_id,
        edits.len(),
        allow_steam_kill
    );

    let guard =
        crate::core::game_mutations::acquire(&steam_root, app_id, "achievements-save").await?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        commit_achievement_changes_blocking(
            &steam_root,
            &data_root,
            app_id,
            account_id,
            &edits,
            allow_steam_kill,
            now_unix_seconds(),
            default_steam_gate,
        )
    })
    .await
    .map_err(|err| format!("Achievement save task failed: {err}"))?
}
