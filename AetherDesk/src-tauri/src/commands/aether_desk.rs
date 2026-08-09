use crate::updater::github::GithubReleaseManager;
use tauri_plugin_updater::UpdaterExt;

#[tauri::command]
pub async fn check_aether_desk_update(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let current_version = app.package_info().version.to_string();
    let manager = GithubReleaseManager::new();
    let release = match manager.fetch_latest_desk_release().await {
        Ok(release) => release,
        Err(error) => {
            return Ok(serde_json::json!({
                "installed_version": current_version,
                "latest_version": "N/A",
                "latest_tag": "N/A",
                "update_available": false,
                "release_url": "",
                "notes": "",
                "error": error
            }));
        }
    };

    let info = GithubReleaseManager::build_desk_update_info(current_version, &release);
    serde_json::to_value(info).map_err(|e| format!("Failed to serialize desk update info: {}", e))
}

#[tauri::command]
pub async fn install_aether_desk_update(app: tauri::AppHandle) -> Result<String, String> {
    let current_version = app.package_info().version.to_string();
    let manager = GithubReleaseManager::new();
    let release = manager.fetch_latest_desk_release().await?;

    if !GithubReleaseManager::latest_is_newer_than(&current_version, &release.tag_name) {
        return Ok(format!("AetherDesk is already up to date ({})", current_version));
    }

    let manifest_url = GithubReleaseManager::find_desk_updater_manifest_url(&release)?;
    let manifest_url = url::Url::parse(&manifest_url)
        .map_err(|e| format!("Invalid updater endpoint URL: {}", e))?;

    let update = app
        .updater_builder()
        .endpoints(vec![manifest_url])
        .map_err(|e| format!("Invalid updater endpoint: {}", e))?
        .build()
        .map_err(|e| format!("Failed to initialize Tauri updater: {}", e))?
        .check()
        .await
        .map_err(|e| format!("Tauri updater check failed: {}", e))?;

    let Some(update) = update else {
        return Err(format!(
            "GitHub tag {} says an update exists, but latest.json did not expose a newer signed Tauri artifact. Check that latest.json version matches the tag semver and that updater artifacts were uploaded.",
            release.tag_name
        ));
    };

    let version = update.version.clone();
    let mut downloaded = 0;
    update
        .download_and_install(
            |chunk_length, content_length| {
                downloaded += chunk_length;
                println!("AetherDesk updater downloaded {downloaded} / {content_length:?} bytes");
            },
            || {
                println!("AetherDesk updater download finished");
            },
        )
        .await
        .map_err(|e| format!("Failed to download/install AetherDesk update: {}", e))?;

    println!("AetherDesk {} installed. Restarting...", version);
    app.restart()
}

#[tauri::command]
pub fn uninstall_aether_desk(app: tauri::AppHandle) -> Result<(), String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.to_path_buf()))
        .ok_or_else(|| "Failed to resolve AetherDesk install directory.".to_string())?;

    let uninstaller = exe_dir.join("uninstall.exe");
    if !uninstaller.is_file() {
        return Err(format!(
            "AetherDesk uninstaller was not found at {}",
            uninstaller.display()
        ));
    }

    std::process::Command::new(&uninstaller)
        .current_dir(&exe_dir)
        .spawn()
        .map_err(|e| format!("Failed to launch AetherDesk uninstaller: {}", e))?;

    app.exit(0);
    Ok(())
}
