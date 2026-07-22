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

use hubcap_client::HubcapClient;
use steam_compat::SteamCompat;
use download_orchestrator::DownloadOrchestrator;
use settings::{AppSettings, SettingsManager};
use store_service::{StoreService, UnifiedStoreGame};
use github_updater::GithubReleaseManager;
use dll_installer::DllInstaller;

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

// Command 5: Trigger first download option (Hubcap LUA pipeline)
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

// Command 6: Kill and Restart Steam process using custom configured path
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

// Command 8: Check if Steam updates are currently blocked
#[tauri::command]
fn is_steam_blocked(steam_path: String) -> Result<bool, String> {
    if steam_path.trim().is_empty() {
        return Ok(false);
    }
    let cfg_path = std::path::PathBuf::from(steam_path).join("steam.cfg");
    if !cfg_path.exists() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(cfg_path)
        .map_err(|e| format!("Failed to read steam.cfg: {}", e))?;
    
    Ok(content.contains("BootStrapperInhibitAll=Enable"))
}

// Command 9: Install / Update AetherDLL from latest GitHub Release
#[tauri::command]
async fn install_aether_dll(steam_path: String) -> Result<String, String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    // 1. Fetch latest release info from michelegoku3/Aether
    let manager = GithubReleaseManager::new();
    let (tag_name, download_url) = manager.fetch_latest_release().await?;

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
    let installer = DllInstaller::new(steam_path);
    let install_result = installer.install_from_zip(&temp_zip_path);

    // Clean up temporary ZIP
    let _ = std::fs::remove_file(temp_zip_path);

    // Propagate result
    install_result.map(|_| format!("AetherDLL {} successfully installed into Steam!", tag_name))
}

// Command 10: Uninstall AetherDLL files from Steam folder
#[tauri::command]
fn uninstall_aether_dll(steam_path: String) -> Result<String, String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }

    let installer = DllInstaller::new(steam_path);
    installer.uninstall().map(|_| "AetherDLL files removed successfully from Steam.".to_string())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            validate_hubcap_key,
            search_store,
            trigger_hubcap_download,
            restart_steam,
            is_dll_installed,
            is_steam_blocked,
            install_aether_dll,
            uninstall_aether_dll,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
