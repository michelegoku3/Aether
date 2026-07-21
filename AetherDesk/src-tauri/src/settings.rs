use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppSettings {
    pub hubcap_api_key: String,
    pub steam_path: String,
    pub active_library: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hubcap_api_key: String::new(),
            steam_path: "C:\\Program Files (x86)\\Steam".to_string(),
            active_library: String::new(),
        }
    }
}

pub struct SettingsManager {
    config_dir: PathBuf,
}

impl SettingsManager {
    pub fn new(app_handle: &tauri::AppHandle) -> Self {
        // Tauri automatically resolves the correct AppData folder on Windows:
        // %APPDATA%\com.aether.desk\
        let config_dir = app_handle
            .path()
            .app_config_dir()
            .unwrap_or_else(|_| PathBuf::from("C:\\Aether\\"));
            
        Self { config_dir }
    }

    fn get_file_path(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    pub fn load(&self) -> AppSettings {
        let path = self.get_file_path();
        if !path.exists() {
            return AppSettings::default();
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return AppSettings::default(),
        };

        serde_json::from_str::<AppSettings>(&content).unwrap_or_else(|_| AppSettings::default())
    }

    pub fn save(&self, settings: &AppSettings) -> Result<(), String> {
        if !self.config_dir.exists() {
            fs::create_dir_all(&self.config_dir)
                .map_err(|e| format!("Failed to create config folder: {}", e))?;
        }

        let path = self.get_file_path();
        let temp_path = path.with_extension("tmp");

        let json_data = serde_json::to_string_pretty(settings)
            .map_err(|e| format!("Serialization error: {}", e))?;

        // Atomic write to prevent file corruption
        fs::write(&temp_path, json_data)
            .map_err(|e| format!("Failed to write temp settings: {}", e))?;
            
        fs::rename(&temp_path, &path)
            .map_err(|e| format!("Failed to apply settings: {}", e))?;

        Ok(())
    }
}
