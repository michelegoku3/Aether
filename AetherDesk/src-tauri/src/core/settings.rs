use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use crate::core::paths::LocalAppPaths;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppSettings {
    #[serde(default)]
    pub hubcap_api_key: String,
    #[serde(default = "default_steam_path")]
    pub steam_path: String,
    #[serde(default)]
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
    /// Depotbox build-details access token. Empty = use the built-in default
    /// (the service key shipped by SFF). Override when SFF rotates its key;
    /// the `AETHERDESK_BUILD_TOKEN` environment variable takes precedence.
    #[serde(default)]
    pub build_details_token: String,
    /// When true, latest-version downloads comment setManifestid pins after
    /// installing the Lua so Steam can keep the game updated. Specific-version
    /// downloads intentionally ignore this setting.
    #[serde(default = "default_true")]
    pub download_games_with_updates_on: bool,
    /// Show a Steam Store front page in Store when no search query is active.
    #[serde(default = "default_true")]
    pub show_store_front_games: bool,
    /// Enables the alternate backdrop-focused game card layout.
    #[serde(default)]
    pub use_alternative_game_cards: bool,
    /// Enables WebView developer tools when supported by the build/runtime.
    #[serde(default)]
    pub enable_webview_devtools: bool,
    /// When true, testing releases (`tdesk-*` / `tdll-*`) are also considered
    /// for updates and take priority over stable ones. Off by default so normal
    /// users never see testing builds. The UI shows these as red update dots.
    #[serde(default)]
    pub enable_test_updates: bool,
    /// Criterion used by the Store front page (`trending`, `latest`, ...).
    #[serde(default = "default_store_front_filter")]
    pub store_front_filter: String,
    /// Preferred Steam store currency for prices shown in Store/Info.
    /// Values are intentionally small and map to Steam country codes:
    /// `eur` -> IT, `usd` -> US, `jpy` -> JP.
    #[serde(default = "default_store_currency")]
    pub store_currency: String,
    /// Personal wallpaper displayed behind AetherDesk content.
    #[serde(default)]
    pub personal_wallpaper_enabled: bool,
    /// Wallpaper image opacity percentage (0..=100).
    #[serde(default = "default_wallpaper_opacity")]
    pub personal_wallpaper_opacity: u8,
    /// Explicitly chosen wallpaper file name inside `config/wallpapers/`.
    /// Empty means "use the first detected wallpaper" (sorted by name).
    #[serde(default)]
    pub wallpaper_selected_file: String,
    /// Explicitly chosen theme file name inside `config/themes/`.
    /// Empty means "use the first detected theme" (sorted by name).
    #[serde(default)]
    pub theme_selected_file: String,
    /// Use a custom window icon from `config/icons/`.
    #[serde(default)]
    pub custom_icon_enabled: bool,
    /// Explicitly chosen icon file name inside `config/icons/`.
    #[serde(default)]
    pub icon_selected_file: String,
    /// Backdrop image opacity (0..=100) of the alternative game cards.
    #[serde(default = "default_alt_cards_opacity")]
    pub alternative_cards_opacity: u8,
    /// Backdrop fade-out toward the bottom (0..=100) of the alternative game cards.
    #[serde(default = "default_alt_cards_fade")]
    pub alternative_cards_fade: u8,
    /// Library install-status filter cycle:
    /// `all` (default) | `installed` | `not_installed`.
    /// Driven by the square play/x toggle next to Refresh in Library.
    #[serde(default = "default_library_install_filter")]
    pub library_install_filter: String,
    /// Custom game name displayed to friends on Steam (game_extra_info).
    #[serde(default)]
    pub custom_game_name: String,
}

/// Serde default provider for the alt-cards backdrop opacity.
fn default_alt_cards_opacity() -> u8 {
    100
}

/// Serde default provider for the alt-cards bottom fade.
fn default_alt_cards_fade() -> u8 {
    20
}

/// Serde default provider for boolean settings that ship enabled.
fn default_true() -> bool {
    true
}

fn default_steam_path() -> String {
    "C:\\Program Files (x86)\\Steam".to_string()
}

fn default_store_currency() -> String {
    "eur".to_string()
}

fn default_store_front_filter() -> String {
    "upcoming".to_string()
}

fn default_wallpaper_opacity() -> u8 {
    20
}

fn default_library_install_filter() -> String {
    "all".to_string()
}

pub fn normalize_library_install_filter(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "installed" => "installed".to_string(),
        "not_installed" | "not-installed" | "uninstalled" => "not_installed".to_string(),
        _ => "all".to_string(),
    }
}

/// Migrate legacy meme icon file name (`aether.ico` → `aether_genshin.ico`).
pub fn normalize_icon_selected_file(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("aether.ico") {
        "aether_genshin.ico".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn normalize_store_currency(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "usd" => "usd".to_string(),
        "jpy" => "jpy".to_string(),
        _ => "eur".to_string(),
    }
}

pub fn steam_country_code_for_currency(value: &str) -> &'static str {
    match normalize_store_currency(value).as_str() {
        "usd" => "US",
        "jpy" => "JP",
        _ => "IT",
    }
}

pub fn cache_version_with_currency(app_version: &str, currency: &str) -> String {
    format!("{}|currency={}", app_version, normalize_store_currency(currency))
}

pub fn normalize_store_front_filter(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "latest" => "latest".to_string(),
        "top_sellers" | "topsellers" => "top_sellers".to_string(),
        "upcoming" => "upcoming".to_string(),
        "popular_upcoming" | "popularcomingsoon" => "popular_upcoming".to_string(),
        "discounts" | "specials" => "discounts".to_string(),
        _ => "trending".to_string(),
    }
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
            build_details_token: String::new(),
            download_games_with_updates_on: true,
            show_store_front_games: true,
            use_alternative_game_cards: false,
            enable_webview_devtools: false,
            enable_test_updates: false,
            store_front_filter: default_store_front_filter(),
            store_currency: default_store_currency(),
            personal_wallpaper_enabled: false,
            personal_wallpaper_opacity: default_wallpaper_opacity(),
            wallpaper_selected_file: String::new(),
            theme_selected_file: String::new(),
            custom_icon_enabled: false,
            icon_selected_file: String::new(),
            alternative_cards_opacity: default_alt_cards_opacity(),
            alternative_cards_fade: default_alt_cards_fade(),
            library_install_filter: default_library_install_filter(),
            custom_game_name: String::new(),
        }
    }
}

pub struct SettingsManager {
    config_dir: PathBuf,
    legacy_config_dir: Option<PathBuf>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProviderCredentials {
    #[serde(default)]
    hubcap_api_key: String,
    #[serde(default)]
    ryuu_api_key: String,
}

impl SettingsManager {
    pub fn new(app_handle: &tauri::AppHandle) -> Self {
        let manager = Self {
            config_dir: LocalAppPaths::data_root_for_app(app_handle).join("config"),
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

    fn credentials_path(&self) -> PathBuf {
        self.config_dir.join("provider_credentials.dat")
    }

    fn load_credentials(&self) -> Option<ProviderCredentials> {
        let encrypted = fs::read(self.credentials_path()).ok()?;
        let plain = crate::core::secure_storage::unprotect(&encrypted).ok()?;
        serde_json::from_slice(&plain).ok()
    }

    fn save_credentials(&self, credentials: &ProviderCredentials) -> Result<(), String> {
        let plain = serde_json::to_vec(credentials)
            .map_err(|error| format!("Failed to serialize provider credentials: {error}"))?;
        let encrypted = crate::core::secure_storage::protect(&plain)?;
        crate::external_tools::fs::write_atomic(&self.credentials_path(), &encrypted)
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

        let mut settings =
            serde_json::from_str::<AppSettings>(&content).unwrap_or_else(|_| AppSettings::default());
        if let Some(credentials) = self.load_credentials() {
            settings.hubcap_api_key = credentials.hubcap_api_key;
            settings.ryuu_api_key = credentials.ryuu_api_key;
        }
        settings
    }

    pub fn save(&self, settings: &AppSettings) -> Result<(), String> {
        if !self.config_dir.exists() {
            fs::create_dir_all(&self.config_dir)
                .map_err(|e| format!("Failed to create local settings folder next to AetherDesk: {}", e))?;
        }

        let path = self.get_file_path();
        let temp_path = path.with_extension("tmp");

        let credentials = ProviderCredentials {
            hubcap_api_key: settings.hubcap_api_key.clone(),
            ryuu_api_key: settings.ryuu_api_key.clone(),
        };
        self.save_credentials(&credentials)?;

        let mut normalized = settings.clone();
        // Secrets are returned to the UI at runtime for the existing settings
        // experience, but are never persisted in plaintext settings.json.
        normalized.hubcap_api_key.clear();
        normalized.ryuu_api_key.clear();
        normalized.store_currency = normalize_store_currency(&normalized.store_currency);
        normalized.store_front_filter = normalize_store_front_filter(&normalized.store_front_filter);
        normalized.library_install_filter =
            normalize_library_install_filter(&normalized.library_install_filter);
        normalized.icon_selected_file =
            normalize_icon_selected_file(&normalized.icon_selected_file);
        normalized.personal_wallpaper_opacity = normalized.personal_wallpaper_opacity.min(100);
        normalized.alternative_cards_opacity = normalized.alternative_cards_opacity.min(100);
        normalized.alternative_cards_fade = normalized.alternative_cards_fade.min(100);

        let json_data = serde_json::to_string_pretty(&normalized)
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
