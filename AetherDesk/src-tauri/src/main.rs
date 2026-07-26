#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod settings;
mod hubcap_client;
mod steam_compat;
mod steam_store;
mod store_service;
mod github_updater;
mod dll_installer;
mod download_orchestrator;
mod steam_update_guard;
mod drm_detector;
mod lua_manifest_pins;

use hubcap_client::HubcapClient;
use steam_compat::SteamCompat;
use download_orchestrator::DownloadOrchestrator;
use settings::{AppSettings, SettingsManager};
use store_service::{StoreService, UnifiedStoreGame};
use github_updater::GithubReleaseManager;
use dll_installer::DllInstaller;
use steam_update_guard::SteamUpdateGuard;
use lua_manifest_pins::{LuaManifestEdit, LuaManifestPins};
use drm_detector::DrmDetector;
use tauri_plugin_updater::UpdaterExt;

// Command 1: Get App Settings (Load from settings.json)
#[tauri::command]
fn get_settings(app: tauri::AppHandle) -> Result<AppSettings, String> {
    let manager = SettingsManager::new(&app);
    Ok(manager.load())
}

// Command 2: Save App Settings (Save to settings.json)
#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: AppSettings) -> Result<(), String> {
    let manager = SettingsManager::new(&app);
    manager.save(&settings)
}

// Command 3: Validate Hubcap API Key (Decoupled & Stateless)
#[tauri::command]
async fn validate_hubcap_key(api_key: String) -> Result<bool, String> {
    if api_key.trim().is_empty() {
        return Err("API Key cannot be empty".to_string());
    }
    
    let client = HubcapClient::new(api_key);
    client.validate_api_key().await
}

// Command 4: Unified Store Search (Steam Catalog + Hubcap Manifest Merge)
#[tauri::command]
async fn search_store(app: tauri::AppHandle, query: String) -> Result<Vec<UnifiedStoreGame>, String> {
    let manager = SettingsManager::new(&app);
    let settings = manager.load();

    let hubcap_client = if !settings.hubcap_api_key.trim().is_empty() {
        Some(HubcapClient::new(settings.hubcap_api_key))
    } else {
        None
    };

    let service = StoreService::new();
    service.search_store(&query, hubcap_client).await
}

// Command 5: Enrich already-rendered store results with Denuvo information.
// This is deliberately separate from search_store so the first results appear quickly.
#[tauri::command]
async fn check_denuvo_bulk(app_ids: Vec<u32>) -> Result<std::collections::HashMap<u32, bool>, String> {
    DrmDetector::new().detect_many(app_ids).await
}

// Command 6: Trigger first download option (Hubcap LUA pipeline)
#[tauri::command]
async fn trigger_hubcap_download(
    app_id: u32,
    api_key: String,
    steam_path: String,
) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err("API Key is required to call Hubcap Manifest".to_string());
    }
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    // Instantiate isolated services
    let client = HubcapClient::new(api_key);
    let steam = SteamCompat::new(steam_path);
    let orchestrator = DownloadOrchestrator::new(client, steam);

    // Run clean download pipeline
    orchestrator.execute_hubcap_download(app_id).await
}

// Command 6: Download and install the Lua file, then return editable setManifestid rows.
// This is used by "Download Specific Version": the Lua is installed normally first,
// then the frontend opens the version-selection table for the installed file.
#[tauri::command]
async fn prepare_specific_version_download(
    app_id: u32,
    api_key: String,
    steam_path: String,
) -> Result<Vec<lua_manifest_pins::LuaManifestRow>, String> {
    if api_key.trim().is_empty() {
        return Err("API Key is required to download the Lua file".to_string());
    }
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    let client = HubcapClient::new(api_key);
    let lua_content = client.download_lua_config(app_id).await?;
    let manifest_rows = LuaManifestPins::rows_from_content(&lua_content);

    // Safety guard: never overwrite the user's installed Lua with a source file that
    // does not contain setManifestid pins when the user explicitly requested the
    // specific-version editor. Without this guard, a provider response that only
    // contains addappid lines would erase all pinned versions from stplug-in.
    if manifest_rows.is_empty() {
        return Err(
            "The downloaded Lua does not contain any setManifestid entries, so it was not installed. Try another source or verify the provider returned the full Lua with manifests.".to_string()
        );
    }

    let steam = SteamCompat::new(steam_path.clone());
    steam.install_lua_config(app_id, &lua_content)?;

    let installed_rows = LuaManifestPins::new(steam_path, app_id).rows_from_file()?;
    if installed_rows.len() != manifest_rows.len() {
        return Err(format!(
            "Lua install verification failed: downloaded file had {} setManifestid entries, installed file has {}.",
            manifest_rows.len(),
            installed_rows.len()
        ));
    }

    Ok(installed_rows)
}

// Command 7: Parse an already-installed Lua file and return editable setManifestid rows.
// This makes the same specific-version modal reusable from the future Library/Installed games view.
#[tauri::command]
fn get_installed_lua_manifest_rows(
    app_id: u32,
    steam_path: String,
) -> Result<Vec<lua_manifest_pins::LuaManifestRow>, String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    LuaManifestPins::new(steam_path, app_id).rows_from_file()
}

// Command 8: Apply edited setManifestid values and depot enable/disable switches.
#[tauri::command]
fn apply_specific_version_edits(
    app_id: u32,
    steam_path: String,
    edits: Vec<LuaManifestEdit>,
) -> Result<Vec<lua_manifest_pins::LuaManifestRow>, String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    LuaManifestPins::new(steam_path, app_id).apply_edits(edits)
}

// Command 8: Kill and Restart Steam process using custom configured path
#[tauri::command]
fn restart_steam(app: tauri::AppHandle) -> Result<(), String> {
    // 1. Terminate any running Steam processes
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes();
    
    let mut terminated = false;
    for process in sys.processes().values() {
        let name = process.name().to_lowercase();
        if name == "steam.exe" || name == "steam" {
            let _ = process.kill();
            terminated = true;
        }
    }

    // Brief delay to release locked file handles on exit
    if terminated {
        std::thread::sleep(std::time::Duration::from_millis(600));
    }

    // 2. Load custom Steam directory path from settings
    let manager = SettingsManager::new(&app);
    let settings = manager.load();
    let steam_dir = std::path::PathBuf::from(&settings.steam_path);

    if !steam_dir.exists() {
        return Err("Steam installation path does not exist. Please check your settings.".to_string());
    }

    let steam_exe = steam_dir.join("steam.exe");
    if !steam_exe.exists() {
        return Err(format!("steam.exe was not found in Steam directory: {:?}", steam_exe));
    }

    // 3. Launch steam.exe asynchronously
    let mut cmd = std::process::Command::new(&steam_exe);
    cmd.current_dir(&steam_dir);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    cmd.spawn().map_err(|e| format!("Failed to launch Steam process: {}", e))?;

    Ok(())
}

// Command 7: Verify if AetherDLL is currently installed in Steam
#[tauri::command]
fn is_dll_installed(steam_path: String) -> Result<bool, String> {
    if steam_path.trim().is_empty() {
        return Ok(false);
    }
    let installer = DllInstaller::new(steam_path);
    Ok(installer.verify_installation())
}

// Command 8: Check if Steam updates are currently blocked.
// If steam.cfg does not exist, it is created with updates still unblocked.
#[tauri::command]
fn is_steam_blocked(steam_path: String) -> Result<bool, String> {
    if steam_path.trim().is_empty() {
        return Ok(false);
    }

    SteamUpdateGuard::new(steam_path).is_blocked()
}

// Command 9: Block Steam client updates by persisting BootStrapperInhibitAll=Enable in steam.cfg
#[tauri::command]
fn block_steam_updates(steam_path: String) -> Result<String, String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    SteamUpdateGuard::new(steam_path).block_updates()?;
    Ok("Steam updates are now blocked.".to_string())
}

// Command 10: Unblock Steam client updates by persisting BootStrapperInhibitAll=Disable in steam.cfg
#[tauri::command]
fn unblock_steam_updates(steam_path: String) -> Result<String, String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    SteamUpdateGuard::new(steam_path).unblock_updates()?;
    Ok("Steam updates are now unblocked.".to_string())
}

// Command 11: Query GitHub and local file system to check for available AetherDLL updates
#[tauri::command]
async fn check_aether_dll_update(steam_path: String) -> Result<serde_json::Value, String> {
    if steam_path.trim().is_empty() {
        return Ok(serde_json::json!({
            "installed_version": "N/A",
            "latest_version": "N/A",
            "update_available": false
        }));
    }

    // 1. Read local installed version from steam directory
    let version_path = std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt");
    let installed_version = if version_path.exists() {
        std::fs::read_to_string(&version_path)
            .unwrap_or_else(|_| "N/A".to_string())
            .trim()
            .to_string()
    } else {
        // Fallback: If DLLs exist on disk but there is no version tracking file, treat as v2.4.1
        let installer = DllInstaller::new(steam_path.clone());
        if installer.verify_installation() {
            "v2.4.1".to_string()
        } else {
            "N/A".to_string()
        }
    };

    // 2. Fetch latest DLL release tag from GitHub. This ignores desk-* tags.
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

    // 3. Compare normalized component versions (dll-1.2.3, dll-v1.2.3 and v1.2.3 all match 1.2.3)
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

// Command 10: Install / Update AetherDLL from latest GitHub Release
#[tauri::command]
async fn install_aether_dll(steam_path: String) -> Result<String, String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    // 1. Fetch latest AetherDLL release info from michelegoku3/Aether.
    // This uses only dll-* / dll-v* tags and will never install an AetherDesk release.
    let manager = GithubReleaseManager::new();
    let (tag_name, download_url) = manager.fetch_latest_dll_release().await?;

    // 2. Download the release ZIP asynchronously
    let client = reqwest::Client::new();
    let response = client.get(&download_url)
        .header("User-Agent", "AetherDesk-Downloader")
        .send()
        .await
        .map_err(|e| format!("Failed to reach download server: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download server returned HTTP error: {}", response.status()));
    }

    let bytes = response.bytes().await
        .map_err(|e| format!("Failed to read downloaded bytes: {}", e))?;

    // 3. Save to temporary file
    let temp_zip_path = std::env::temp_dir().join("aether_dll_latest.zip");
    std::fs::write(&temp_zip_path, &bytes)
        .map_err(|e| format!("Failed to write temporary ZIP: {}", e))?;

    // 4. Extract and deploy DLLs using DllInstaller
    let installer = DllInstaller::new(steam_path.clone());
    let install_result = installer.install_from_zip(&temp_zip_path);

    // Clean up temporary ZIP
    let _ = std::fs::remove_file(temp_zip_path);

    // If install succeeded, write version tag to a local file to track installed state!
    if install_result.is_ok() {
        let version_path = std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt");
        let _ = std::fs::write(&version_path, &tag_name);
    }

    // Propagate result
    install_result.map(|_| format!("AetherDLL {} successfully installed into Steam!", tag_name))
}

// Command 11: Check if a native AetherDesk application update is available.
// Detection is tag-based: it reads the latest published desk-* / desk-v* GitHub Release,
// not the version field inside latest.json. latest.json is used only later by Tauri to
// verify, download and install the signed updater artifact.
#[tauri::command]
async fn check_aether_desk_update(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
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

// Command 12: Install AetherDesk update using Tauri's native updater.
// The release is still selected by desk-* / desk-v* tag first. Once selected, we point
// the Tauri updater to that release's latest.json asset, so signature verification and
// installer execution stay native and safe.
#[tauri::command]
async fn install_aether_desk_update(app: tauri::AppHandle) -> Result<String, String> {
    let current_version = app.package_info().version.to_string();
    let manager = GithubReleaseManager::new();
    let release = manager.fetch_latest_desk_release().await?;

    if !GithubReleaseManager::tags_are_different_versions(&current_version, &release.tag_name) {
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

    // Native restart: Tauri relaunches into the newly installed version.
    // app.restart() does not return, so it must be the final expression of this command.
    app.restart()
}

// Command 13: Uninstall AetherDLL files from Steam folder
#[tauri::command]
fn uninstall_aether_dll(steam_path: String) -> Result<String, String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    // Delete version file
    let version_path = std::path::PathBuf::from(&steam_path).join("AetherDLL_version.txt");
    let _ = std::fs::remove_file(version_path);

    let installer = DllInstaller::new(steam_path);
    installer.uninstall().map(|_| "AetherDLL files removed successfully from Steam.".to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            validate_hubcap_key,
            search_store,
            check_denuvo_bulk,
            trigger_hubcap_download,
            prepare_specific_version_download,
            get_installed_lua_manifest_rows,
            apply_specific_version_edits,
            restart_steam,
            is_dll_installed,
            is_steam_blocked,
            block_steam_updates,
            unblock_steam_updates,
            check_aether_dll_update,
            install_aether_dll,
            check_aether_desk_update,
            install_aether_desk_update,
            uninstall_aether_dll,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
