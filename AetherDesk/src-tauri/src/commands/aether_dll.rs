use crate::app_storage::AppStorage;
use crate::dll_installer::DllInstaller;
use crate::github_updater::GithubReleaseManager;

#[tauri::command]
pub async fn check_aether_dll_update(app: tauri::AppHandle, steam_path: String) -> Result<serde_json::Value, String> {
    if steam_path.trim().is_empty() {
        return Ok(serde_json::json!({
            "installed_version": "N/A",
            "latest_version": "N/A",
            "update_available": false
        }));
    }

    let storage = AppStorage::new(&app);
    let legacy_version_path = std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt");
    let installed_version = if let Some(version) = storage.read_aether_dll_version() {
        let _ = std::fs::remove_file(&legacy_version_path);
        version
    } else if legacy_version_path.exists() {
        let version = std::fs::read_to_string(&legacy_version_path)
            .unwrap_or_else(|_| "N/A".to_string())
            .trim()
            .to_string();
        if version != "N/A" {
            let _ = storage.write_aether_dll_version(&version);
        }
        let _ = std::fs::remove_file(&legacy_version_path);
        version
    } else if DllInstaller::new(steam_path.clone()).verify_installation() {
        "v2.4.1".to_string()
    } else {
        "N/A".to_string()
    };

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

    let update_available = installed_version != "N/A"
        && latest_tag != "N/A"
        && GithubReleaseManager::tags_are_different_versions(&installed_version, &latest_tag);

    Ok(serde_json::json!({
        "installed_version": installed_version,
        "latest_version": latest_version,
        "latest_tag": latest_tag,
        "update_available": update_available
    }))
}

#[tauri::command]
pub async fn install_aether_dll(app: tauri::AppHandle, steam_path: String) -> Result<String, String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    let manager = GithubReleaseManager::new();
    let (tag_name, download_url) = manager.fetch_latest_dll_release().await?;

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
        AppStorage::new(&app).write_aether_dll_version(&tag_name)?;
        let legacy_version_path = std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt");
        let _ = std::fs::remove_file(legacy_version_path);
    }

    install_result.map(|_| format!("AetherDLL {} successfully installed into Steam!", tag_name))
}

#[tauri::command]
pub fn uninstall_aether_dll(app: tauri::AppHandle, steam_path: String) -> Result<String, String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    AppStorage::new(&app).remove_aether_dll_version();
    let legacy_version_path = std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt");
    let _ = std::fs::remove_file(legacy_version_path);

    DllInstaller::new(steam_path)
        .uninstall()
        .map(|_| "AetherDLL files removed successfully from Steam.".to_string())
}
