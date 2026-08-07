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
    /// When false, rows tagged NSFW (Steam sexual content descriptors or name
    /// heuristic) are filtered out of store search results. Default TRUE
    /// (visible, pink border): the custom serde default is required because
    /// `#[serde(default)]` alone would read missing fields as `false` when
    /// parsing older settings.json files.
    #[serde(default = "default_true")]
    pub show_store_nsfw: bool,
    /// When false, rows Steam flags as `unlisted` (delisted games) are
    /// filtered out of store search results. Default TRUE (visible, white
    /// border): unlisted classics like GTA SA or Dark Souls PTDE are exactly
    /// what people search a manifest tool for.
    #[serde(default = "default_true")]
    pub show_store_delisted: bool,
    /// When true, the frontend injects `AetherData/config/custom.css` as a
    /// `<style id="aether-custom-css">` after the default theme.
    /// Default `false` — no file I/O unless the user opts in.
    #[serde(default)]
    pub custom_css_enabled: bool,
    /// Ryuu API key for `generator.ryuu.lol`. No validation endpoint,
    /// so an empty string means "not configured" and any non-empty is saved verbatim.
    /// Limit is 50 uses per day (enforced server-side).
    #[serde(default)]
    pub ryuu_api_key: String,
}

/// Serde default provider for boolean settings that ship enabled.
fn default_true() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hubcap_api_key: String::new(),
            steam_path: "C:\\Program Files (x86)\\Steam".to_string(),
            active_library: String::new(),
            antivirus_exclusion_done: false,
            show_store_dlcs: false,
            show_store_nsfw: true,
            show_store_delisted: true,
            custom_css_enabled: false,
            ryuu_api_key: String::new(),
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
        // The migration logic itself lives in `core::migration` (single home
        // for all migration helpers); calling it here keeps SettingsManager
        // self-sufficient even when used before the startup hub runs.
        crate::core::migration::migrate_legacy_settings_if_needed(
            &manager.config_dir,
            manager.legacy_config_dir.as_deref(),
        );
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

}
