use crate::hubcap_client::HubcapClient;
use crate::settings::{AppSettings, SettingsManager};

#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> Result<AppSettings, String> {
    let manager = SettingsManager::new(&app);
    Ok(manager.load())
}

#[tauri::command]
pub fn save_settings(app: tauri::AppHandle, settings: AppSettings) -> Result<(), String> {
    let manager = SettingsManager::new(&app);
    manager.save(&settings)
}

#[tauri::command]
pub async fn validate_hubcap_key(api_key: String) -> Result<bool, String> {
    if api_key.trim().is_empty() {
        return Err("API Key cannot be empty".to_string());
    }

    HubcapClient::new(api_key).validate_api_key().await
}
