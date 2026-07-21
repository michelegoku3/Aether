#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod settings;
mod hubcap_client;
mod steam_compat;
mod download_orchestrator;

use hubcap_client::HubcapClient;
use steam_compat::SteamCompat;
use download_orchestrator::DownloadOrchestrator;
use settings::{AppSettings, SettingsManager};

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

// Command 4: Trigger first download option (Hubcap LUA pipeline)
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

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            validate_hubcap_key,
            trigger_hubcap_download,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
