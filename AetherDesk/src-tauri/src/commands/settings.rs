use crate::providers::hubcap::HubcapClient;
use crate::core::settings::{AppSettings, SettingsManager};

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

#[tauri::command]
pub async fn get_hubcap_usage(api_key: String) -> Result<serde_json::Value, String> {
    if api_key.trim().is_empty() {
        return Ok(serde_json::json!({ "usage": 0, "limit": 25 }));
    }

    match HubcapClient::new(api_key).get_usage_stats().await {
        Ok(stats) => {
            let limit = stats.role_daily_limit.or(stats.daily_limit).unwrap_or(25);
            let usage = stats.daily_usage.unwrap_or(0);
            Ok(serde_json::json!({
                "usage": usage,
                "limit": limit
            }))
        }
        Err(_) => {
            Ok(serde_json::json!({ "usage": 0, "limit": 25 }))
        }
    }
}
