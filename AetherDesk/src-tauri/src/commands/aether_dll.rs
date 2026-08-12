use crate::core::settings::SettingsManager;
use crate::updater::dll::DllInstaller;
use crate::updater::dll_version::read_installed_dll_version;
use crate::updater::github::GithubReleaseManager;

#[tauri::command]
pub fn get_installed_dll_version(steam_path: String) -> String {
    if steam_path.trim().is_empty() {
        return "N/A".to_string();
    }
    let legacy_version_path = std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt");
    let raw = read_installed_dll_version(std::path::Path::new(&steam_path))
        .unwrap_or_else(|| read_legacy_installed_version(&legacy_version_path, &steam_path));
    GithubReleaseManager::display_version_from_tag(&raw)
}

#[tauri::command]
pub async fn check_aether_dll_update(app: tauri::AppHandle, steam_path: String) -> Result<serde_json::Value, String> {
    if steam_path.trim().is_empty() {
        return Ok(serde_json::json!({
            "installed_version": "N/A",
            "latest_version": "N/A",
            "update_available": false,
            "is_test": false
        }));
    }

    let legacy_version_path = std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt");

    // Fonte di verità primaria: la version resource DENTRO i .dll (scritta a compile
    // time dal build CMake, root CMakeLists.txt) — nessun file esterno coinvolto.
    // Se manca (installazioni precedenti alla feature), catena legacy di sola lettura.
    let installed_version = read_installed_dll_version(std::path::Path::new(&steam_path))
        .unwrap_or_else(|| read_legacy_installed_version(&legacy_version_path, &steam_path));

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

    if SettingsManager::new(&app).load().enable_test_updates {
        crate::desk_log_info!("updater", "Test updates enabled: probing tdll-* first");
        match GithubReleaseManager::new().fetch_latest_dll_test_release().await {
            Ok((tag, url)) => {
                let latest_version = GithubReleaseManager::component_version_from_tag(&tag);
                let update_available =
                    GithubReleaseManager::latest_is_newer_than(&installed_version, &tag);
                crate::desk_log_info!(
                    "updater",
                    "AetherDLL TEST check: installed={} latest={} tag={} url={} update_available={}",
                    installed_version,
                    latest_version,
                    tag,
                    url,
                    update_available
                );
                return Ok(serde_json::json!({
                    "installed_version": GithubReleaseManager::display_version_from_tag(&installed_version),
                    "latest_version": GithubReleaseManager::display_version_from_tag(&latest_version),
                    "latest_tag": tag,
                    "update_available": update_available,
                    "is_test": true
                }));
            }
            Err(error) => {
                crate::desk_log_warn!(
                    "updater",
                    "No usable tdll-* release ({}). Falling through to stable dll-*",
                    error
                );
            }
        }
    }

    let manager = GithubReleaseManager::new();
    let (latest_tag, download_url) = match manager.fetch_latest_dll_release().await {
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

/// Catena legacy **di sola lettura** per installazioni pre-resource (le DLL non hanno
/// la versione dentro): bookmark residuo nella cartella Steam → sola presenza file
/// ("?"). Non viene più scritto/letto NULLA in AetherData: la cartella
/// `component_versions` è ritirata e la migrazione di avvio la elimina.
fn read_legacy_installed_version(legacy_version_path: &std::path::Path, steam_path: &str) -> String {
    if legacy_version_path.exists() {
        std::fs::read_to_string(legacy_version_path)
            .unwrap_or_else(|_| "N/A".to_string())
            .trim()
            .to_string()
    } else if DllInstaller::new(steam_path.to_string()).verify_installation() {
        // DLL presenti ma NESSUNA fonte di versione attendibile (né resource PE nei
        // file, né bookmark legacy): MAI inventare un numero — il vecchio "v2.4.1"
        // cablato qui mentiva (ed era pure obsoleto). "?" rende onesto l'ignoto:
        // la UI mostra "v?" e, dato che != ultimo tag, propone l'update — che a sua
        // volta installa una build con la versione leggibile dentro i .dll.
        "?".to_string()
    } else {
        "N/A".to_string()
    }
}

#[tauri::command]
pub async fn install_aether_dll(app: tauri::AppHandle, steam_path: String) -> Result<String, String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    ensure_steam_is_closed()?;
    crate::desk_log_info!("updater", "Starting installation of AetherDLL into steam_path='{}'", steam_path);

    let manager = GithubReleaseManager::new();

    // Testing releases take priority when enabled.
    let (tag_name, download_url) = if SettingsManager::new(&app).load().enable_test_updates {
        match manager.fetch_latest_dll_test_release().await {
            Ok(pair) => {
                crate::desk_log_info!("updater", "Install will use TEST DLL tag {}", pair.0);
                pair
            }
            Err(error) => {
                crate::desk_log_warn!(
                    "updater",
                    "TEST DLL release unavailable ({}). Using stable dll-*",
                    error
                );
                manager.fetch_latest_dll_release().await?
            }
        }
    } else {
        manager.fetch_latest_dll_release().await?
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
pub fn uninstall_aether_dll(_app: tauri::AppHandle, steam_path: String) -> Result<String, String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    ensure_steam_is_closed()?;
    crate::desk_log_info!("updater", "Uninstalling AetherDLL from Steam directory '{}'", steam_path);

    // Rimuove l'eventuale bookmark residuo nella dir Steam (i .dll li elimina
    // l'installer qui sotto; in AetherData non viene più scritto nulla).
    let legacy_version_path = std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt");
    let _ = std::fs::remove_file(legacy_version_path);

    DllInstaller::new(steam_path)
        .uninstall()
        .map(|_| "AetherDLL files removed successfully from Steam.".to_string())
}

#[tauri::command]
pub fn reset_aether_steam_path(_app: tauri::AppHandle, steam_path: String) -> Result<String, String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }
    ensure_steam_is_closed()?;
    crate::desk_log_info!("updater", "Resetting Aether files in Steam directory '{}'", steam_path);

    let legacy_version_path = std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt");
    let _ = std::fs::remove_file(legacy_version_path);

    let removed = DllInstaller::new(steam_path.clone()).reset_aether_files()?;
    crate::desk_log_info!("updater", "Steam path reset completed: removed {} item(s) from '{}'", removed, steam_path);
    Ok(format!(
        "Steam path reset completed. Removed {} Aether-created item(s).",
        removed
    ))
}

fn ensure_steam_is_closed() -> Result<(), String> {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes();
    let steam_running = sys.processes().values().any(|process| {
        let name = process.name().to_lowercase();
        name == "steam.exe" || name == "steam"
    });

    if steam_running {
        Err("Steam is currently running. Close Steam completely before installing, uninstalling, or resetting AetherDLL files.".to_string())
    } else {
        Ok(())
    }
}
