use crate::game_info::model::GameInfo;
use crate::game_info::service::GameInfoService;

#[tauri::command]
pub async fn get_game_info(app: tauri::AppHandle, app_id: u32) -> Result<GameInfo, String> {
    GameInfoService::new(app).get_game_info(app_id).await
}
