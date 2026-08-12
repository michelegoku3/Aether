use crate::game_info::model::GameInfo;
use crate::game_info::service::GameInfoService;

#[tauri::command]
pub async fn get_game_info(app: tauri::AppHandle, app_id: u32) -> Result<GameInfo, String> {
    crate::desk_log_debug!("game_info", "Fetching game info for AppID {}", app_id);
    let res = GameInfoService::new(app).get_game_info(app_id).await;
    match &res {
        Ok(info) => crate::desk_log_info!("game_info", "Successfully fetched game info for AppID {}: '{}'", app_id, info.name.as_deref().unwrap_or("Unknown")),
        Err(e) => crate::desk_log_warn!("game_info", "Failed to fetch game info for AppID {}: {}", app_id, e),
    }
    res
}
