use crate::core::settings::SettingsManager;
use crate::updater::dll::DllInstaller;
use crate::updater::dll_version::read_installed_dll_version;
use crate::updater::github::GithubReleaseManager;

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
    if SettingsManager::new(&app).load().enable_test_updates {
        if let Ok((tag, _)) = GithubReleaseManager::new().fetch_latest_dll_test_release().await {
            let latest_version = GithubReleaseManager::component_version_from_tag(&tag);
            let update_available =
                GithubReleaseManager::latest_is_newer_than(&installed_version, &tag);
            return Ok(serde_json::json!({
                "installed_version": GithubReleaseManager::display_version_from_tag(&installed_version),
                "latest_version": GithubReleaseManager::display_version_from_tag(&latest_version),
                "latest_tag": tag,
                "update_available": update_available,
                "is_test": true
            }));
        }
    }

    let manager = GithubReleaseManager::new();
    let latest_tag = match manager.fetch_latest_dll_release().await {
        Ok((tag, _)) => tag,
        Err(_) => "N/A".to_string(),
    };
    let latest_version = if latest_tag != "N/A" {
        GithubReleaseManager::component_version_from_tag(&latest_tag)
    } else {
        "N/A".to_string()
    };

    let update_available = GithubReleaseManager::latest_is_newer_than(&installed_version, &latest_tag);

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

    let manager = GithubReleaseManager::new();

    // Determine installed DLL version so we can gate test releases by version too.
    let installed_version = read_installed_dll_version(std::path::Path::new(&steam_path))
        .unwrap_or_else(|| {
            read_legacy_installed_version(
                &std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt"),
                &steam_path,
            )
        });

    // Testing releases take priority when enabled.
    let (tag_name, download_url) = if SettingsManager::new(&app).load().enable_test_updates {
        match manager.fetch_latest_dll_test_release().await {
            Ok(pair) => pair,
            Err(_) => manager.fetch_latest_dll_release().await?,
        }
    } else {
        manager.fetch_latest_dll_release().await?
    };

    let response = reqwest::Client::new()
        .get(&download_url)
        .header("User-Agent", "AetherDesk-Downloader")
        .send()
        .await
        .map_err(|e| format!("Failed to reach download server: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download server returned HTTP error: {}", response.status()));
    }

    let bytes = response.bytes().await
        .map_err(|e| format!("Failed to read downloaded bytes: {}", e))?;

    let temp_zip_path = std::env::temp_dir().join("aether_dll_latest.zip");
    std::fs::write(&temp_zip_path, &bytes)
        .map_err(|e| format!("Failed to write temporary ZIP: {}", e))?;

    let installer = DllInstaller::new(steam_path.clone());
    let install_result = installer.install_from_zip(&temp_zip_path);
    let _ = std::fs::remove_file(temp_zip_path);

    if install_result.is_ok() {
        // Nessun file di versione esterno: la versione vive dentro i .dll (version
        // resource PE). Rimuoviamo solo l'eventuale bookmark residuo nella dir Steam.
        let legacy_version_path = std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt");
        let _ = std::fs::remove_file(legacy_version_path);

        return Ok(format!("AetherDLL {} successfully installed into Steam!", tag_name));
    }

    install_result.map(|_| format!("AetherDLL {} successfully installed into Steam!", tag_name))
}

#[tauri::command]
pub fn uninstall_aether_dll(_app: tauri::AppHandle, steam_path: String) -> Result<String, String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    ensure_steam_is_closed()?;

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

    let legacy_version_path = std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt");
    let _ = std::fs::remove_file(legacy_version_path);

    let removed = DllInstaller::new(steam_path).reset_aether_files()?;
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
