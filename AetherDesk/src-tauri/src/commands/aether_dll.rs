use crate::core::settings::load_settings;
use crate::steam::resolve::resolve_steam_path;
use crate::updater::dll::DllInstaller;
use crate::updater::dll_version::read_installed_dll_version;
use crate::updater::github::{self, GithubReleaseManager, TestChannel};
use super::{command_steam_path, configured_steam_path};

#[tauri::command]
pub fn get_installed_dll_version(app: tauri::AppHandle) -> String {
    // Percorso dalle impostazioni, non dal client. Se manca o Steam non è
    // raggiungibile la risposta resta "N/A": il pannello DLL deve poter dire
    // "non installato" senza che diventi un errore per l'utente.
    let steam_path = configured_steam_path(&app).unwrap_or_default();
    if resolve_steam_path(&steam_path).is_err() {
        return "N/A".to_string();
    }
    let raw = read_installed_dll_version(std::path::Path::new(&steam_path))
        .unwrap_or_else(|| "N/A".to_string());
    GithubReleaseManager::display_version_from_tag(&raw)
}

#[tauri::command]
pub async fn check_aether_dll_update(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let steam_path = configured_steam_path(&app).unwrap_or_default();
    // An unreachable Steam root means "unknown", never "update available":
    // without this, installed="N/A" compares as older than any tag and shows
    // a spurious update badge (plus pointless GitHub calls).
    if resolve_steam_path(&steam_path).is_err() {
        return Ok(serde_json::json!({
            "installed_version": "N/A",
            "latest_version": "N/A",
            "update_available": false,
            "is_test": false
        }));
    }

    // The only supported installation has three coherent PE version resources.
    let installed_version = read_installed_dll_version(std::path::Path::new(&steam_path))
        .unwrap_or_else(|| "N/A".to_string());

    // Origin label: the update check is reachable from the startup pass, the
    // DLL-change path and the manual Check button; without it a duplicated
    // check was untraceable in the logs (F7).
    let origin = "dll-check";
    // Testing releases (`tdll-*`) take priority when enabled. Their version is
    // gated by `latest_is_newer_than`, exactly like stable releases: if the test
    // release is not newer than installed, `update_available` is false and no dot
    // is shown, without falling through to the stable stream.
    crate::desk_log_info!(
        "updater",
        "Checking for AetherDLL updates (installed={}, steam_path='{}')",
        installed_version,
        steam_path
    );

    if load_settings(&app).enable_test_updates {
        if let Some(response) = test_channel_update_response(&installed_version, origin).await {
            return Ok(response);
        }
    }

    let manager = GithubReleaseManager::new();
    let (latest_tag, download_url) = match manager.fetch_latest_dll_release("dll-install").await {
        Ok(pair) => pair,
        Err(error) => {
            crate::desk_log_error!("updater", "AetherDLL update check failed: {}", error);
            return Err(error);
        }
    };
    let latest_version = GithubReleaseManager::component_version_from_tag(&latest_tag);
    let update_available = GithubReleaseManager::latest_is_newer_than(&installed_version, &latest_tag);
    crate::desk_log_info!(
        "updater",
        "AetherDLL check: installed={} latest={} tag={} url={} update_available={}",
        installed_version,
        latest_version,
        latest_tag,
        download_url,
        update_available
    );

    Ok(serde_json::json!({
        "installed_version": GithubReleaseManager::display_version_from_tag(&installed_version),
        "latest_version": GithubReleaseManager::display_version_from_tag(&latest_version),
        "latest_tag": latest_tag,
        "update_available": update_available,
        "is_test": false
    }))
}

/// Result of probing the `tdll-*` test channel during an update check.
///
/// `Some(response)` means the test channel published a release and the check is
/// finished. `None` means "fall through to the stable `dll-*` channel", which
/// covers all three legitimate reasons: the test channel published nothing, it
/// is known to be empty in this session, or its lookup failed for a reason that
/// is not the user's problem to solve.
async fn test_channel_update_response(
    installed_version: &str,
    origin: &str,
) -> Option<serde_json::Value> {
    // A session-level memo skips a probe that cannot answer yet: on an install
    // with no test build published, every check used to spend two GitHub API
    // requests to learn the same "not published" (F7). A published release
    // clears the memo immediately, so this never delays a real test build.
    if github::test_channel_recently_empty(TestChannel::Dll) {
        crate::desk_log_info!(
            "updater",
            "tdll-* channel known empty (checked recently): using the stable dll-* channel"
        );
        return None;
    }

    crate::desk_log_info!("updater", "Test updates enabled: probing tdll-* first");
    match GithubReleaseManager::new()
        .fetch_latest_dll_test_release(origin)
        .await
    {
        Ok((tag, url)) => {
            let latest_version = GithubReleaseManager::component_version_from_tag(&tag);
            let update_available =
                GithubReleaseManager::latest_is_newer_than(installed_version, &tag);
            crate::desk_log_info!(
                "updater",
                "AetherDLL TEST check: installed={} latest={} tag={} url={} update_available={}",
                installed_version,
                latest_version,
                tag,
                url,
                update_available
            );
            Some(serde_json::json!({
                "installed_version": GithubReleaseManager::display_version_from_tag(installed_version),
                "latest_version": GithubReleaseManager::display_version_from_tag(&latest_version),
                "latest_tag": tag,
                "update_available": update_available,
                "is_test": true
            }))
        }
        Err(error) if github::channel_has_no_release(&error) => {
            // Expected whenever no test build is published: not a fault.
            crate::desk_log_info!(
                "updater",
                "No tdll-* release published ({}). Using the stable dll-* channel.",
                error
            );
            None
        }
        Err(error) => {
            crate::desk_log_warn!(
                "updater",
                "No usable tdll-* release ({}). Falling through to stable dll-*",
                error
            );
            None
        }
    }
}

#[tauri::command]
pub async fn install_aether_dll(
    app: tauri::AppHandle,
    origin: Option<String>,
) -> Result<String, String> {
    let origin = origin.unwrap_or_else(|| "dll-install".to_string());
    // Strict validation first: fail fast before any download or Steam-side write.
    // Il percorso viene dalle impostazioni ed è già validato qui dentro.
    let steam_path = command_steam_path(&app)?;
    crate::desk_log_info!(
        "updater",
        "AetherDLL install requested (origin={}, steam_path='{}')",
        origin,
        steam_path
    );

    ensure_steam_is_closed()?;
    crate::desk_log_info!("updater", "Starting installation of AetherDLL into steam_path='{}'", steam_path);

    let manager = GithubReleaseManager::new();

    // Testing releases take priority when enabled.
    let (tag_name, download_url) = if load_settings(&app).enable_test_updates {
        match manager.fetch_latest_dll_test_release("dll-install").await {
            Ok(pair) => {
                crate::desk_log_info!("updater", "Install will use TEST DLL tag {}", pair.0);
                pair
            }
            Err(error) if crate::updater::github::channel_has_no_release(&error) => {
                crate::desk_log_info!(
                    "updater",
                    "No tdll-* release published ({}). Installing from the stable dll-* channel.",
                    error
                );
                manager.fetch_latest_dll_release("dll-install").await?
            }
            Err(error) => {
                crate::desk_log_warn!(
                    "updater",
                    "TEST DLL release unavailable ({}). Using stable dll-*",
                    error
                );
                manager.fetch_latest_dll_release("dll-install").await?
            }
        }
    } else {
        manager.fetch_latest_dll_release("dll-install").await?
    };

    crate::desk_log_info!("updater", "Downloading AetherDLL release tag {} from {}", tag_name, download_url);

    let response = reqwest::Client::new()
        .get(&download_url)
        .header("User-Agent", "AetherDesk-Downloader")
        .send()
        .await
        .map_err(|e| {
            crate::desk_log_error!("updater", "AetherDLL download network error: {}", e);
            format!("Failed to reach download server: {}", e)
        })?;

    crate::desk_log_info!("updater", "AetherDLL download HTTP {}", response.status());
    if !response.status().is_success() {
        crate::desk_log_error!(
            "updater",
            "AetherDLL download failed: HTTP {} from {}",
            response.status(),
            download_url
        );
        return Err(format!("Download server returned HTTP error: {}", response.status()));
    }

    let bytes = response.bytes().await
        .map_err(|e| {
            crate::desk_log_error!("updater", "AetherDLL download body error: {}", e);
            format!("Failed to read downloaded bytes: {}", e)
        })?;
    crate::desk_log_info!("updater", "AetherDLL zip size={} bytes", bytes.len());

    let temp_zip_path = std::env::temp_dir().join("aether_dll_latest.zip");
    std::fs::write(&temp_zip_path, &bytes)
        .map_err(|e| format!("Failed to write temporary ZIP: {}", e))?;

    let installer = DllInstaller::new(steam_path.clone());
    let install_result = installer.install_from_zip(&temp_zip_path);
    let _ = std::fs::remove_file(temp_zip_path);

    match install_result {
        Ok(()) => {
            let legacy_version_path = std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt");
            let _ = std::fs::remove_file(legacy_version_path);
            crate::desk_log_info!(
                "updater",
                "AetherDLL {} successfully installed into Steam directory '{}'",
                tag_name,
                steam_path
            );
            Ok(format!("AetherDLL {} successfully installed into Steam!", tag_name))
        }
        Err(error) => {
            crate::desk_log_error!(
                "updater",
                "AetherDLL {} install failed in '{}': {}",
                tag_name,
                steam_path,
                error
            );
            Err(error)
        }
    }
}

#[tauri::command]
pub fn uninstall_aether_dll(app: tauri::AppHandle) -> Result<String, String> {
    let steam_path = command_steam_path(&app)?;

    ensure_steam_is_closed()?;
    crate::desk_log_info!("updater", "Uninstalling AetherDLL from Steam directory '{}'", steam_path);

    // The uninstaller also removes the obsolete Steam-root proxy.
    DllInstaller::new(steam_path.clone()).uninstall()?;
    let legacy_version_path = std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt");
    let _ = std::fs::remove_file(legacy_version_path);
    Ok("AetherDLL files removed successfully from Steam.".to_string())
}

#[tauri::command]
pub fn reset_aether_steam_path(app: tauri::AppHandle) -> Result<String, String> {
    let steam_path = command_steam_path(&app)?;
    ensure_steam_is_closed()?;
    crate::desk_log_info!("updater", "Resetting Aether files in Steam directory '{}'", steam_path);

    // Reset removes the obsolete proxy and version bookmark too.
    let removed = DllInstaller::new(steam_path.clone()).reset_aether_files()?;
    crate::desk_log_info!("updater", "Steam path reset completed: removed {} item(s) from '{}'", removed, steam_path);
    Ok(format!(
        "Steam path reset completed. Removed {} Aether-created item(s).",
        removed
    ))
}

/// Reports residual Aether artifacts, including the obsolete Steam proxy.
/// Reset Path removes the proxy as part of normal Aether cleanup.
/// Safe while Steam is running — read-only probe for the Uninstall confirm UI.
#[tauri::command]
pub fn probe_aether_steam_residuals(app: tauri::AppHandle) -> Result<usize, String> {
    let steam_path = configured_steam_path(&app).unwrap_or_default();
    if resolve_steam_path(&steam_path).is_err() {
        return Ok(0);
    }
    let count = DllInstaller::new(steam_path).count_aether_residuals();
    crate::desk_log_info!(
        "lifecycle",
        "Probed Aether Steam residuals: {} item(s)",
        count
    );
    Ok(count)
}

fn ensure_steam_is_closed() -> Result<(), String> {
    // Do not rely on the cached UI monitor for destructive file changes.
    if crate::core::steam_process::is_steam_running_fresh() {
        Err("Steam is currently running. Close Steam completely before installing, uninstalling, or resetting AetherDLL files.".to_string())
    } else {
        Ok(())
    }
}
