use crate::providers::hubcap::HubcapClient;
use crate::providers::luatools_auth::{LuaToolsAuth, LuaToolsAuthStatus};
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
    manager.save(&settings)?;
    if let Err(e) = crate::core::custom_css::apply_window_icon(&app) {
        crate::desk_log_warn!("settings", "Window icon apply after save failed: {}", e);
    }
    sync_custom_game_name_to_aethercore_toml(&app, &settings.custom_game_name);
    Ok(())
}

fn sync_custom_game_name_to_aethercore_toml(app: &tauri::AppHandle, custom_name: &str) {
    let config_dir = crate::core::paths::LocalAppPaths::config_dir();
    let toml_path = config_dir.join("aethercore.toml");
    update_custom_name_in_toml(&toml_path, custom_name);

    let steam_path = crate::core::settings::SettingsManager::new(app).load().steam_path;
    if !steam_path.trim().is_empty() {
        let steam_toml = std::path::PathBuf::from(&steam_path).join("aethercore").join("aethercore.toml");
        update_custom_name_in_toml(&steam_toml, custom_name);
    }
}

fn update_custom_name_in_toml(path: &std::path::Path, custom_name: &str) {
    if !path.exists() {
        return;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let escaped = custom_name.replace('\\', "\\\\").replace('"', "\\\"");
    let new_line = format!("custom_game_name = \"{}\"", escaped);

    let updated = if content.contains("[presence]") {
        let re = regex::Regex::new(r#"(?m)^custom_game_name\s*=\s*".*""#).unwrap();
        if re.is_match(&content) {
            re.replace(&content, new_line).to_string()
        } else {
            content.replace("[presence]", &format!("[presence]\n{}", new_line))
        }
    } else {
        format!("{}\n\n[presence]\n{}\n", content.trim_end(), new_line)
    };
    let _ = std::fs::write(path, updated);
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
pub fn get_luatools_auth_status() -> Result<LuaToolsAuthStatus, String> {
    Ok(LuaToolsAuth::new().status())
}

#[tauri::command]
pub async fn sign_in_luatools() -> Result<LuaToolsAuthStatus, String> {
    crate::desk_log_info!("luatools", "Starting LuaTools Discord PKCE sign-in");
    let result = LuaToolsAuth::new().sign_in().await;
    match &result {
        Ok(status) => crate::desk_log_info!(
            "luatools",
            "LuaTools sign-in completed for {}",
            status
                .display_name
                .as_deref()
                .or(status.email.as_deref())
                .unwrap_or("account")
        ),
        Err(error) => crate::desk_log_error!("luatools", "LuaTools sign-in failed: {}", error),
    }
    result
}

#[tauri::command]
pub fn cancel_luatools_sign_in() {
    LuaToolsAuth::cancel_sign_in();
    crate::desk_log_info!("luatools", "LuaTools OAuth sign-in cancelled by user");
}

#[tauri::command]
pub async fn sign_in_luatools_with_code(code: String) -> Result<LuaToolsAuthStatus, String> {
    crate::desk_log_info!("luatools", "Redeeming privacy-oriented @Luie login code");
    let result = LuaToolsAuth::new().sign_in_with_code(&code).await;
    match &result {
        Ok(_) => crate::desk_log_info!("luatools", "LuaTools code sign-in completed"),
        Err(error) => crate::desk_log_error!(
            "luatools",
            "LuaTools code sign-in failed: {}",
            error
        ),
    }
    result
}

#[tauri::command]
pub fn sign_out_luatools() -> Result<(), String> {
    LuaToolsAuth::new().sign_out()?;
    crate::desk_log_info!("luatools", "LuaTools session removed");
    Ok(())
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
