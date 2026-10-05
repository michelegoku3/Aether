use crate::core::paths::LocalAppPaths;
use crate::core::settings::{AppSettings, SettingsManager};
use crate::providers::hubcap::HubcapClient;
use crate::providers::hubcap_generation::{
    MAX_GAME_GENERATIONS_PER_DAY, MAX_WORKSHOP_GENERATIONS_PER_DAY,
};
use crate::providers::luatools_auth::{LuaToolsAuth, LuaToolsAuthStatus};

#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> Result<AppSettings, String> {
    SettingsManager::new(&app).try_load()
}

#[tauri::command]
pub async fn save_settings(
    app: tauri::AppHandle,
    settings: AppSettings,
    base: AppSettings,
) -> Result<AppSettings, String> {
    let manager = SettingsManager::new(&app);
    let previous = manager.try_load()?;
    let candidate = crate::core::settings::merge_settings(&base, &settings, &previous)?;
    if candidate.download_games_with_updates_on {
        if candidate.hubcap_api_key.trim().is_empty() {
            return Err("HUBCAP_KEY_REQUIRED_FOR_UPDATES: enable updates only with a valid active Hubcap key".into());
        }
        let active = HubcapClient::new(candidate.hubcap_api_key.clone())
            .validate_api_key()
            .await
            .map_err(|e| format!("HUBCAP_KEY_REQUIRED_FOR_UPDATES: {e}"))?;
        if !active {
            return Err("HUBCAP_KEY_REQUIRED_FOR_UPDATES: the Hubcap key is not active".into());
        }
    }
    manager.update("ui-patch", |current| {
        let next = crate::core::settings::merge_settings(&base, &settings, current)?;
        if next.download_games_with_updates_on
            && (!candidate.download_games_with_updates_on
                || next.hubcap_api_key != candidate.hubcap_api_key)
        {
            return Err(
                "SETTINGS_CONFLICT: update policy/key changed during validation; retry".into(),
            );
        }
        *current = next;
        Ok(())
    })?;
    apply_post_save_effects(&app, &previous.steam_path)?;
    manager.try_load()
}

#[tauri::command]
pub fn reset_settings_to_defaults(app: tauri::AppHandle) -> Result<AppSettings, String> {
    let manager = SettingsManager::new(&app);
    let mut previous_root = String::new();
    manager.update("reset-defaults", |current| {
        previous_root = current.steam_path.clone();
        let mut defaults = AppSettings::default();
        defaults.library_install_filter = current.library_install_filter.clone();
        defaults.antivirus_exclusion_done = current.antivirus_exclusion_done;
        defaults.ost_warning_acknowledged = current.ost_warning_acknowledged;
        defaults.download_updates_default_off_migrated =
            current.download_updates_default_off_migrated;
        *current = defaults;
        Ok(())
    })?;
    apply_post_save_effects(&app, &previous_root)?;
    manager.try_load()
}

/// Serialize side effects and re-read the committed truth, never an older
/// command's snapshot. A failed effect is reported as a PARTIAL save.
fn apply_post_save_effects(app: &tauri::AppHandle, previous_root: &str) -> Result<(), String> {
    static EFFECTS: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let mut last_root = EFFECTS.lock().map_err(|_| "Settings effects unavailable")?;
    let settings = SettingsManager::new(app).try_load()?;
    if last_root.as_deref().unwrap_or(previous_root).trim() != settings.steam_path.trim() {
        crate::core::library_events::reconfigure_library_watch(app, &settings.steam_path);
        crate::core::library_events::notify_lua_changed(
            app,
            crate::core::library_events::LibraryChangeOrigin::Settings,
            std::iter::empty::<u32>(),
        );
    }
    *last_root = Some(settings.steam_path.clone());
    crate::core::migration::ensure_aethercore_bridge(app);
    if let Err(e) = crate::core::custom_css::apply_window_icon(app) {
        crate::desk_log_warn!("settings", "Window icon apply after commit failed: {}", e);
    }
    for path in crate::core::presence_config::aethercore_toml_paths(app) {
        crate::core::config_document::set_value(
            &path,
            "presence",
            "custom_game_name",
            toml_edit::Value::from(settings.custom_game_name.as_str()),
        )
        .map_err(|e| format!("Settings committed, DLL configuration sync incomplete: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn validate_hubcap_key(api_key: String) -> Result<bool, String> {
    if api_key.trim().is_empty() {
        return Err("API Key cannot be empty".to_string());
    }

    crate::desk_log_info!(
        "settings",
        "Validating Hubcap API key with hubcapmanifest.com..."
    );
    let res = HubcapClient::new(api_key).validate_api_key().await;
    match &res {
        Ok(true) => crate::desk_log_info!("settings", "Hubcap API key validated successfully"),
        Ok(false) => crate::desk_log_warn!(
            "settings",
            "Hubcap API key validation returned false (invalid key)"
        ),
        Err(e) => crate::desk_log_error!(
            "settings",
            "Hubcap API key validation request failed: {}",
            e
        ),
    }
    res
}

/// How long the Hubcap usage counters are believed before asking again.
///
/// `/status/{id}` is a metadata endpoint (it does not count against the daily
/// generation quota), but the account is rate-limited: in a real session log
/// Desk issued the same request twice within 64 ms — the startup badge and the
/// Settings mount — and the provider answered `429 Too Many Requests`. The
/// number only moves when a generation runs, so a minute of reuse is free.
const USAGE_SNAPSHOT_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Last successful usage answer, keyed by the credential it was fetched with
/// (a different key means a different account: never reuse across keys).
pub(crate) struct UsageSnapshot {
    pub api_key: String,
    pub fetched: std::time::Instant,
    pub payload: serde_json::Value,
}

/// Cache decision, kept pure so it can be tested without a network.
pub(crate) fn usage_snapshot_is_fresh(
    snapshot: &UsageSnapshot,
    api_key: &str,
    now: std::time::Instant,
    ttl: std::time::Duration,
) -> bool {
    snapshot.api_key == api_key && now.saturating_duration_since(snapshot.fetched) < ttl
}

fn usage_snapshot_slot() -> &'static tokio::sync::Mutex<Option<UsageSnapshot>> {
    static SLOT: std::sync::OnceLock<tokio::sync::Mutex<Option<UsageSnapshot>>> =
        std::sync::OnceLock::new();
    SLOT.get_or_init(|| tokio::sync::Mutex::new(None))
}

fn usage_defaults() -> serde_json::Value {
    serde_json::json!({
        "usage": 0,
        "limit": MAX_GAME_GENERATIONS_PER_DAY,
        "workshopLimit": MAX_WORKSHOP_GENERATIONS_PER_DAY,
        "reset": "midnight EST"
    })
}

#[tauri::command]
pub async fn get_hubcap_usage(api_key: String) -> Result<serde_json::Value, String> {
    if api_key.trim().is_empty() {
        return Ok(usage_defaults());
    }

    // Serializing the callers is what makes the second one reuse the first
    // one's answer instead of repeating the request: whoever waits here finds
    // a fresh snapshot and returns immediately.
    let slot = usage_snapshot_slot();
    let mut cached = slot.lock().await;

    if let Some(snapshot) = cached.as_ref() {
        if usage_snapshot_is_fresh(
            snapshot,
            &api_key,
            std::time::Instant::now(),
            USAGE_SNAPSHOT_TTL,
        ) {
            crate::desk_log_debug!("settings", "Hubcap usage served from session cache");
            return Ok(snapshot.payload.clone());
        }
    }

    let previous = cached.as_ref().map(|snapshot| snapshot.payload.clone());
    let same_key_as_previous = cached
        .as_ref()
        .map(|snapshot| snapshot.api_key == api_key)
        .unwrap_or(false);

    match HubcapClient::new(api_key.clone()).get_usage_stats().await {
        Ok(stats) => {
            let limit = stats
                .role_daily_limit
                .or(stats.daily_limit)
                .unwrap_or(MAX_GAME_GENERATIONS_PER_DAY);
            let usage = stats.daily_usage.unwrap_or(0);
            let payload = serde_json::json!({
                "usage": usage,
                "limit": limit,
                "workshopLimit": MAX_WORKSHOP_GENERATIONS_PER_DAY,
                "reset": "midnight EST"
            });
            *cached = Some(UsageSnapshot {
                api_key,
                fetched: std::time::Instant::now(),
                payload: payload.clone(),
            });
            Ok(payload)
        }
        Err(error) => {
            // A stale number beats a fabricated zero: the badge would otherwise
            // claim "0/1500 used" precisely when the provider is unreachable
            // (rate-limited, offline), which reads as "nothing was used".
            if same_key_as_previous {
                if let Some(payload) = previous {
                    crate::desk_log_warn!(
                        "settings",
                        "Hubcap usage request failed; serving the last known snapshot: {}",
                        error
                    );
                    return Ok(payload);
                }
            }
            crate::desk_log_warn!(
                "settings",
                "Hubcap usage request failed; returning local quota defaults: {}",
                error
            );
            Ok(usage_defaults())
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
        Err(error) if error == "LuaTools sign-in cancelled" => {
            crate::desk_log_info!("luatools", "LuaTools sign-in cancelled");
        }
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
        Err(error) => crate::desk_log_error!("luatools", "LuaTools code sign-in failed: {}", error),
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
        std::fs::remove_dir_all(&cache_dir).map_err(|error| {
            format!(
                "Failed to clear cache folder {}: {}",
                cache_dir.display(),
                error
            )
        })?;
    }
    std::fs::create_dir_all(&cache_dir).map_err(|error| {
        format!(
            "Failed to recreate cache folder {}: {}",
            cache_dir.display(),
            error
        )
    })?;

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
