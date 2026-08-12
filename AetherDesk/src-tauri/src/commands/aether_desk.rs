//! Tauri commands for AetherDesk self-management (update / uninstall).
//!
//! These commands are thin wrappers: all update logic lives in
//! [`crate::updater::desk`], keeping this file decoupled and easy to maintain.

use crate::core::settings::SettingsManager;
use crate::updater::desk;
use crate::updater::github::GithubReleaseManager;

/// Reports the installed version and whether an update is available.
///
/// When testing releases are enabled, a `tdesk-*` release takes priority **only
/// if its version is newer** than the installed one. Otherwise it falls back to
/// the latest stable `desk-*` release (version-gated). `is_test` tells the UI
/// how to color the update dot.
#[tauri::command]
pub async fn check_aether_desk_update(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let current_version = app.package_info().version.to_string();
    crate::desk_log_info_once!("updater", "Checking for AetherDesk updates (current: {})", current_version);
    let manager = GithubReleaseManager::new();

    // Testing releases (`tdesk-*`) take priority when enabled. Their version is
    // gated by `latest_is_newer_than`, exactly like stable releases: if the test
    // release is not newer than installed, `update_available` is false and no dot
    // is shown, without falling through to the stable stream.
    if SettingsManager::new(&app).load().enable_test_updates {
        if let Ok(release) = manager.fetch_latest_desk_test_release().await {
            let info =
                GithubReleaseManager::build_desk_test_update_info(current_version, &release);
            return serde_json::to_value(info)
                .map_err(|e| format!("Failed to serialize desk test update info: {e}"));
        }
    }

    let release = match manager.fetch_latest_desk_release().await {
        Ok(release) => release,
        Err(error) => {
            return Ok(serde_json::json!({
                "installed_version": current_version,
                "latest_version": "N/A",
                "latest_tag": "N/A",
                "update_available": false,
                "is_test": false,
                "release_url": "",
                "notes": "",
                "error": error
            }));
        }
    };

    let info = GithubReleaseManager::build_desk_update_info(current_version, &release);
    serde_json::to_value(info).map_err(|e| format!("Failed to serialize desk update info: {e}"))
}

/// Downloads the latest portable ZIP, stages it, and schedules a restart to
/// apply it. The running instance exits so the swap can take place.
#[tauri::command]
pub async fn install_aether_desk_update(app: tauri::AppHandle) -> Result<String, String> {
    crate::desk_log_info!("updater", "Starting download and installation of AetherDesk portable update...");
    let Some(prepared) = desk::prepare_update(&app).await? else {
        crate::desk_log_info!("updater", "AetherDesk portable update check: already up to date");
        return Ok("AetherDesk is already up to date.".to_string());
    };

    crate::desk_log_info!("updater", "AetherDesk update staged at {}; scheduling restart and exiting current instance", prepared.app_root.display());
    desk::schedule_restart(&prepared)?;

    // Exit so the original exe is unlocked and the staged updater can swap files.
    app.exit(0);
    Ok("AetherDesk update downloaded. Restarting to apply...".to_string())
}

/// In portable mode there is no installer/uninstaller: removing AetherDesk is
/// simply deleting its folder. This command just closes the running instance.
#[tauri::command]
pub fn uninstall_aether_desk(app: tauri::AppHandle) -> Result<(), String> {
    crate::desk_log_info!("lifecycle", "AetherDesk uninstall command invoked: closing running instance for portable folder removal.");
    app.exit(0);
    Ok(())
}
