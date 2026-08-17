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
    crate::desk_log_info!(
        "updater",
        "Checking for AetherDesk updates (installed={})",
        current_version
    );
    let manager = GithubReleaseManager::new();

    if SettingsManager::new(&app).load().enable_test_updates {
        crate::desk_log_info!("updater", "Test updates enabled: probing tdesk-* first");
        match manager.fetch_latest_desk_test_release().await {
            Ok(release) => {
                let info =
                    GithubReleaseManager::build_desk_test_update_info(current_version.clone(), &release);
                crate::desk_log_info!(
                    "updater",
                    "AetherDesk TEST check: installed={} latest={} tag={} update_available={}",
                    info.installed_version,
                    info.latest_version,
                    info.latest_tag,
                    info.update_available
                );
                return serde_json::to_value(info)
                    .map_err(|e| format!("Failed to serialize desk test update info: {e}"));
            }
            Err(error) => {
                crate::desk_log_warn!(
                    "updater",
                    "No usable tdesk-* release ({}). Falling through to stable desk-*",
                    error
                );
            }
        }
    }

    let release = match manager.fetch_latest_desk_release().await {
        Ok(release) => release,
        Err(error) => {
            crate::desk_log_error!("updater", "AetherDesk update check failed: {}", error);
            return Err(error);
        }
    };

    let info = GithubReleaseManager::build_desk_update_info(current_version, &release);
    crate::desk_log_info!(
        "updater",
        "AetherDesk check: installed={} latest={} tag={} update_available={}",
        info.installed_version,
        info.latest_version,
        info.latest_tag,
        info.update_available
    );
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

/// Riporta AetherDesk alla build stabile (uscita dal canale test): scarica
/// l'ultima release `desk-*`, la prepara e riavvia l'app per applicarla.
#[tauri::command]
pub async fn restore_stable_desk(app: tauri::AppHandle) -> Result<String, String> {
    crate::desk_log_info!("updater", "Restoring stable AetherDesk build (leaving test channel)...");
    let Some(prepared) = desk::prepare_stable_restore(&app).await? else {
        return Ok("AetherDesk is already on the latest stable build.".to_string());
    };

    crate::desk_log_info!(
        "updater",
        "Stable AetherDesk staged at {}; scheduling restart and exiting current instance",
        prepared.app_root.display()
    );
    desk::schedule_restart(&prepared)?;

    app.exit(0);
    Ok("Stable AetherDesk restored. Restarting to apply...".to_string())
}

/// Portable uninstall:
/// 1. stages an external helper (`aether_uninstaller.exe` in system temp);
/// 2. the helper force-kills leftover desk processes, optionally relocates
///    `AetherData` next to the install folder, then deletes `install_root`;
/// 3. this process hard-exits so WebView2 cannot keep it alive in background.
///
/// Steam cleanup (Reset Path) is orchestrated by the frontend *before* this
/// command when the user confirmed it in the steam-clean modal.
///
/// `delete_user_data`:
/// - `true`  → wipe the whole portable folder including AetherData
/// - `false` → move AetherData to the parent directory, then wipe the folder
#[tauri::command]
pub fn uninstall_aether_desk(_app: tauri::AppHandle, delete_user_data: bool) -> Result<(), String> {
    crate::desk_log_info!(
        "lifecycle",
        "AetherDesk uninstall requested (delete_user_data={})",
        delete_user_data
    );
    desk::schedule_uninstall(delete_user_data)?;
    // Hard exit: `app.exit` alone can leave WebView2 / the process alive long
    // enough that the install folder stays locked (empty folder leftover).
    std::process::exit(0);
}
