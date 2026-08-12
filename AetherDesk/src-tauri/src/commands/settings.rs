use crate::providers::hubcap::HubcapClient;
use crate::core::paths::LocalAppPaths;
use crate::core::settings::{AppSettings, SettingsManager};

#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> Result<AppSettings, String> {
    let manager = SettingsManager::new(&app);
    Ok(manager.load())
}

#[tauri::command]
pub fn save_settings(app: tauri::AppHandle, settings: AppSettings) -> Result<(), String> {
    crate::desk_log_info!("settings", "Saving user settings to disk (steam_path='{}', hubcap_key_set={})",
        settings.steam_path, !settings.hubcap_api_key.trim().is_empty());
    let manager = SettingsManager::new(&app);
    manager.save(&settings)
}

#[tauri::command]
pub async fn validate_hubcap_key(api_key: String) -> Result<bool, String> {
    if api_key.trim().is_empty() {
        return Err("API Key cannot be empty".to_string());
    }

    crate::desk_log_info!("settings", "Validating Hubcap API key with hubcapmanifest.com...");
    let res = HubcapClient::new(api_key).validate_api_key().await;
    match &res {
        Ok(true) => crate::desk_log_info!("settings", "Hubcap API key validated successfully"),
        Ok(false) => crate::desk_log_warn!("settings", "Hubcap API key validation returned false (invalid key)"),
        Err(e) => crate::desk_log_error!("settings", "Hubcap API key validation request failed: {}", e),
    }
    res
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

#[tauri::command]
pub fn clear_app_caches() -> Result<String, String> {
    crate::desk_log_info!("settings", "Clearing AetherDesk cache folder...");
    let cache_dir = LocalAppPaths::data_root().join("cache");
    if cache_dir.is_dir() {
        std::fs::remove_dir_all(&cache_dir)
            .map_err(|error| format!("Failed to clear cache folder {}: {}", cache_dir.display(), error))?;
    }
    std::fs::create_dir_all(&cache_dir)
        .map_err(|error| format!("Failed to recreate cache folder {}: {}", cache_dir.display(), error))?;

    Ok("AetherDesk caches cleared successfully.".to_string())
}

#[tauri::command]
pub fn open_webview_devtools(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let Some(window) = app.get_webview_window("main") else {
        return Err("Main WebView window was not found.".to_string());
    };
    window.open_devtools();
    Ok(())
}
