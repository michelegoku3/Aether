use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use crate::core::paths::LocalAppPaths;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppSettings {
    pub hubcap_api_key: String,
    pub steam_path: String,
    pub active_library: String,
    /// Set to true once the user has been asked (and handled) the Windows
    /// Defender exclusion prompt, so it never shows again (install or update).
    /// `#[serde(default)]` keeps old settings.json files parseable.
    #[serde(default)]
    pub antivirus_exclusion_done: bool,
    /// When false (default), DLC-like rows are filtered out of store search
    /// results (SFF structural rule set via batched Steam GetItems). When true,
    /// the Hubcap-only tail is shown unfiltered. `#[serde(default)]` keeps old
    /// settings.json files parseable and preserves the "hidden" default.
    #[serde(default)]
    pub show_store_dlcs: bool,
    /// When false (default), rows tagged NSFW (Steam sexual content descriptors
    /// or name heuristic) are filtered out of store search results. When true
    /// they stay visible, marked with a pink border in the UI.
    #[serde(default)]
    pub show_store_nsfw: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hubcap_api_key: String::new(),
            steam_path: "C:\\Program Files (x86)\\Steam".to_string(),
            active_library: String::new(),
            antivirus_exclusion_done: false,
            show_store_dlcs: false,
            show_store_nsfw: false,
        }
    }
}

pub struct SettingsManager {
    config_dir: PathBuf,
    legacy_config_dir: Option<PathBuf>,
}

impl SettingsManager {
    pub fn new(app_handle: &tauri::AppHandle) -> Self {
        let manager = Self {
            config_dir: LocalAppPaths::config_dir(),
            legacy_config_dir: LocalAppPaths::legacy_app_config_dir(app_handle),
        };
        manager.migrate_legacy_settings_if_needed();
        manager
    }

    fn get_file_path(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    fn get_legacy_file_path(&self) -> Option<PathBuf> {
        self.legacy_config_dir.as_ref().map(|dir| dir.join("settings.json"))
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
                .map_err(|e| format!("Failed to create local settings folder next to AetherDesk: {}", e))?;
        }

        let path = self.get_file_path();
        let temp_path = path.with_extension("tmp");

        let json_data = serde_json::to_string_pretty(settings)
            .map_err(|e| format!("Serialization error: {}", e))?;

        fs::write(&temp_path, json_data)
            .map_err(|e| format!("Failed to write temp settings: {}", e))?;
            
        fs::rename(&temp_path, &path)
            .map_err(|e| format!("Failed to apply settings: {}", e))?;

        if let Some(legacy_path) = self.get_legacy_file_path() {
            let _ = fs::remove_file(legacy_path);
        }

        Ok(())
    }

    fn migrate_legacy_settings_if_needed(&self) {
        let local_path = self.get_file_path();
        if local_path.exists() {
            return;
        }

        let Some(legacy_path) = self.get_legacy_file_path() else {
            return;
        };
        if !legacy_path.exists() {
            return;
        }

        if let Ok(content) = fs::read_to_string(&legacy_path) {
            if let Some(parent) = local_path.parent() {
                if fs::create_dir_all(parent).is_ok() && fs::write(&local_path, content).is_ok() {
                    let _ = fs::remove_file(legacy_path);
                }
            }
        }
    }
}
