use crate::core::paths::LocalAppPaths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct AppSettings {
    #[serde(default)]
    pub hubcap_api_key: String,
    #[serde(default = "default_steam_path")]
    pub steam_path: String,
    // NOTE: no `active_library` field. Library discovery already resolves
    // every folder transitively from `libraryfolders.vdf`, so a separate
    // "active library" setting was dead configuration (never exposed in the
    // UI, always empty). Old settings.json files carrying the key still parse:
    // serde ignores unknown fields by default.
    /// Set to true once the user has been asked (and handled) the Windows
    /// Defender exclusion prompt, so it never shows again (install or update).
    /// `#[serde(default)]` keeps old settings.json files parseable.
    #[serde(default)]
    pub antivirus_exclusion_done: bool,
    /// Set to true once the user has pressed "I understand" on the OST
    /// pattern-source warning, so the popup shows only on first enable.
    /// `#[serde(default)]` keeps old settings.json files parseable.
    #[serde(default)]
    pub ost_warning_acknowledged: bool,
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
    /// downloads intentionally ignore this setting. It is disabled by default
    /// because unauthenticated Steam request-code access no longer provides
    /// reliable automatic updates or Workshop behavior.
    #[serde(default = "default_false")]
    pub download_games_with_updates_on: bool,
    /// When true (default), the periodic Workshop synchronizer asks the
    /// running Steam client to download the actual Workshop payload
    /// (`steam://workshop/downloaditem/...`, one spawn at a time) whenever an
    /// item has a staged manifest but no content directory. When false, the
    /// sync stages manifests only and content downloads stay on-demand —
    /// the "no startup-storm" mode after a depotcache wipe.
    #[serde(default = "default_true")]
    pub workshop_auto_download_content: bool,
    /// Records that the one-time safe-default migration for the update policy
    /// has been applied. This lives in settings.json so the migration does not
    /// depend on a standalone filesystem sentinel.
    #[serde(default)]
    pub download_updates_default_off_migrated: bool,
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

fn default_false() -> bool {
    false
}

fn default_steam_path() -> String {
    // Fresh installs get the auto-detected installation when one exists,
    // else the historical location (see `steam::resolve`).
    crate::steam::resolve::default_steam_path()
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

/// Major Steam store currencies (ISO 4217, lowercase) offered by the UI.
/// Unknown values fall back to EUR everywhere (normalize + country map +
/// cache key), so the frontend can never poison the store queries.
pub fn normalize_store_currency(value: &str) -> String {
    let code = value.trim().to_lowercase();
    match code.as_str() {
        "eur" | "usd" | "gbp" | "jpy" | "ars" | "brl" | "cad" | "aud" | "chf" | "cny" | "krw"
        | "inr" | "mxn" | "rub" | "try" | "pln" | "sek" | "nok" | "dkk" | "nzd" | "sgd" | "hkd"
        | "twd" | "thb" | "myr" | "idr" | "php" | "ils" | "aed" | "sar" | "clp" | "cop" | "pen"
        | "uah" | "kzt" | "vnd" | "zar" => code,
        _ => "eur".to_string(),
    }
}

pub fn steam_country_code_for_currency(value: &str) -> &'static str {
    match normalize_store_currency(value).as_str() {
        "usd" => "US",
        "gbp" => "GB",
        "jpy" => "JP",
        "ars" => "AR",
        "brl" => "BR",
        "cad" => "CA",
        "aud" => "AU",
        "chf" => "CH",
        "cny" => "CN",
        "krw" => "KR",
        "inr" => "IN",
        "mxn" => "MX",
        "rub" => "RU",
        "try" => "TR",
        "pln" => "PL",
        "sek" => "SE",
        "nok" => "NO",
        "dkk" => "DK",
        "nzd" => "NZ",
        "sgd" => "SG",
        "hkd" => "HK",
        "twd" => "TW",
        "thb" => "TH",
        "myr" => "MY",
        "idr" => "ID",
        "php" => "PH",
        "ils" => "IL",
        "aed" => "AE",
        "sar" => "SA",
        "clp" => "CL",
        "cop" => "CO",
        "pen" => "PE",
        "uah" => "UA",
        "kzt" => "KZ",
        "vnd" => "VN",
        "zar" => "ZA",
        _ => "IT", // "eur" + anything unknown
    }
}

pub fn cache_version_with_currency(app_version: &str, currency: &str) -> String {
    format!(
        "{}|currency={}",
        app_version,
        normalize_store_currency(currency)
    )
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
            steam_path: default_steam_path(),
            antivirus_exclusion_done: false,
            ost_warning_acknowledged: false,
            show_store_dlcs: false,
            show_store_nsfw: true,
            show_store_delisted: true,
            custom_css_enabled: false,
            ryuu_api_key: String::new(),
            build_details_token: String::new(),
            download_games_with_updates_on: false,
            workshop_auto_download_content: true,
            download_updates_default_off_migrated: false,
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
        let _guard = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    fn pending_path(&self) -> PathBuf {
        self.config_dir.join("settings.pending.json")
    }

    /// Complete an interrupted two-file commit before exposing either half.
    /// Pending contains only sanitized JSON and DPAPI ciphertext, never keys.
    fn recover_locked(&self) -> Result<(), String> {
        let bytes = match fs::read(self.pending_path()) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("Cannot read settings recovery record: {e}")),
        };
        let pending: PendingSettings = serde_json::from_slice(&bytes)
            .map_err(|_| "Settings recovery record is corrupt; refusing overwrite".to_string())?;
        if pending.format_version != 1 {
            return Err("Unsupported settings recovery format; refusing overwrite".into());
        }
        crate::desk_log_info!(
            "settings",
            "Applying pending settings transaction (idempotent recovery)"
        );
        // Validate the journal before writing either destination.
        serde_json::from_str::<AppSettings>(&pending.settings)
            .map_err(|_| "Settings recovery payload is invalid".to_string())?;
        super::state_io::write_atomic(&self.credentials_path(), &pending.credentials)?;
        super::state_io::write_atomic(&self.get_file_path(), pending.settings.as_bytes())?;
        fs::remove_file(self.pending_path())
            .map_err(|e| format!("Settings committed but recovery cleanup failed: {e}"))?;
        crate::desk_log_info!(
            "settings",
            "Settings transaction finalized (JSON + protected credentials)"
        );
        Ok(())
    }

    fn load_locked(&self) -> Result<AppSettings, String> {
        self.recover_locked()?;
        let content = match fs::read_to_string(self.get_file_path()) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if self.credentials_path().exists() {
                    return Err("settings.json is missing but credentials exist; refusing first-run defaults".into());
                }
                return Ok(AppSettings::default());
            }
            Err(e) => return Err(format!("Cannot read settings: {e}")),
        };
        let mut settings: AppSettings = serde_json::from_str(&content)
            .map_err(|_| "Invalid settings.json; original preserved (settings.last-good.json is available after a successful update)".to_string())?;
        match fs::read(self.credentials_path()) {
            Ok(encrypted) => {
                let plain = crate::core::secure_storage::unprotect(&encrypted)?;
                let credentials: ProviderCredentials = serde_json::from_slice(&plain)
                    .map_err(|_| "Invalid protected provider credentials".to_string())?;
                settings.hubcap_api_key = credentials.hubcap_api_key;
                settings.ryuu_api_key = credentials.ryuu_api_key;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Cannot read protected credentials: {e}")),
        }
        Ok(settings)
    }

    pub fn try_load(&self) -> Result<AppSettings, String> {
        let _guard = SETTINGS_LOCK
            .lock()
            .map_err(|_| "Settings repository unavailable")?;
        let _file_lock = super::state_io::lock(&self.config_dir.join("settings.lock"))?;
        let result = self.load_locked();
        if let Err(e) = &result {
            crate::desk_log_error!(
                "settings",
                "Load failed; no defaults will be persisted: {}",
                e
            );
        }
        result
    }

    /// Legacy read-only consumers may use defaults, but mutation paths always
    /// use try_load/update and therefore never persist this fallback.
    pub fn load(&self) -> AppSettings {
        self.try_load().unwrap_or_default()
    }

    /// Typed mutation under the repository lock. The closure must be pure:
    /// no network, no nested settings calls and no side effects.
    pub fn update(
        &self,
        operation: &str,
        change: impl FnOnce(&mut AppSettings) -> Result<(), String>,
    ) -> Result<AppSettings, String> {
        let _guard = SETTINGS_LOCK
            .lock()
            .map_err(|_| "Settings repository unavailable")?;
        let _file_lock = super::state_io::lock(&self.config_dir.join("settings.lock"))?;
        let result = (|| {
            let previous = self.load_locked()?;
            let mut next = previous.clone();
            change(&mut next)?;
            normalize_settings(&mut next);
            if next == previous {
                crate::desk_log_debug!("settings", "Mutation unchanged operation={}", operation);
                return Ok(next);
            }
            self.commit_locked(&previous, &next)?;
            crate::desk_log_info!(
                "settings",
                "Mutation committed operation={} (values redacted)",
                operation
            );
            Ok(next)
        })();
        if let Err(e) = &result {
            crate::desk_log_error!("settings", "Mutation failed operation={}: {}", operation, e);
        }
        result
    }

    fn commit_locked(&self, previous: &AppSettings, settings: &AppSettings) -> Result<(), String> {
        let credentials = ProviderCredentials {
            hubcap_api_key: settings.hubcap_api_key.clone(),
            ryuu_api_key: settings.ryuu_api_key.clone(),
        };
        let plain = serde_json::to_vec(&credentials).map_err(|e| e.to_string())?;
        let encrypted = crate::core::secure_storage::protect(&plain)?;
        let mut disk = settings.clone();
        disk.hubcap_api_key.clear();
        disk.ryuu_api_key.clear();
        let pending = PendingSettings {
            format_version: 1,
            settings: serde_json::to_string_pretty(&disk).map_err(|e| e.to_string())?,
            credentials: encrypted,
        };
        // A complete last-good pair (ciphertext + sanitized settings) remains
        // available for manual recovery; never back up plaintext legacy keys.
        if self.get_file_path().exists() {
            let mut old = previous.clone();
            let old_secret = ProviderCredentials {
                hubcap_api_key: old.hubcap_api_key.clone(),
                ryuu_api_key: old.ryuu_api_key.clone(),
            };
            old.hubcap_api_key.clear();
            old.ryuu_api_key.clear();
            let backup = PendingSettings {
                format_version: 1,
                settings: serde_json::to_string_pretty(&old).map_err(|e| e.to_string())?,
                credentials: crate::core::secure_storage::protect(
                    &serde_json::to_vec(&old_secret).map_err(|e| e.to_string())?,
                )?,
            };
            super::state_io::write_atomic(
                &self.config_dir.join("settings.last-good.json"),
                &serde_json::to_vec(&backup).map_err(|e| e.to_string())?,
            )?;
        }
        super::state_io::write_atomic(
            &self.pending_path(),
            &serde_json::to_vec(&pending).map_err(|e| e.to_string())?,
        )?;
        crate::desk_log_debug!(
            "settings",
            "Settings transaction prepared; interrupted writes will roll forward on next access"
        );
        self.recover_locked()
    }
}

/// Convenience: load settings in one call. Replaces the repetitive
/// `SettingsManager::new(app).load()` idiom that was copy-pasted across every
/// command and made review harder; a single call-site also makes it trivial
/// to add caching/memoisation later.
#[inline]
pub fn load_settings(app: &tauri::AppHandle) -> AppSettings {
    SettingsManager::new(app).load()
}

/// Convenience: return a non-empty `steam_path` or a descriptive error ready
/// for `?` propagation. Centralises the "steam path required" guard that
/// dozens of commands reimplemented individually with subtly different
/// phrasing.
#[inline]
pub fn require_steam_path(app: &tauri::AppHandle) -> Result<String, String> {
    let path = SettingsManager::new(app).try_load()?.steam_path;
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(
            "Steam installation path is not configured. Please set it in Settings first."
                .to_string(),
        );
    }
    Ok(crate::steam::resolve::normalize_steam_path(trimmed))
}

static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[derive(Serialize, Deserialize)]
struct PendingSettings {
    format_version: u32,
    settings: String,
    credentials: Vec<u8>,
}

fn normalize_settings(settings: &mut AppSettings) {
    settings.steam_path = crate::steam::resolve::normalize_steam_path(&settings.steam_path);
    settings.store_currency = normalize_store_currency(&settings.store_currency);
    settings.store_front_filter = normalize_store_front_filter(&settings.store_front_filter);
    settings.library_install_filter =
        normalize_library_install_filter(&settings.library_install_filter);
    settings.icon_selected_file = normalize_icon_selected_file(&settings.icon_selected_file);
    settings.personal_wallpaper_opacity = settings.personal_wallpaper_opacity.min(100);
    settings.alternative_cards_opacity = settings.alternative_cards_opacity.min(100);
    settings.alternative_cards_fade = settings.alternative_cards_fade.min(100);
}

pub(crate) fn merge_settings(
    base: &AppSettings,
    requested: &AppSettings,
    current: &AppSettings,
) -> Result<AppSettings, String> {
    let base = serde_json::to_value(base).map_err(|e| e.to_string())?;
    let requested = serde_json::to_value(requested).map_err(|e| e.to_string())?;
    let mut merged = serde_json::to_value(current).map_err(|e| e.to_string())?;
    for (key, value) in requested.as_object().ok_or("Invalid settings patch")? {
        if base.get(key) == Some(value) {
            continue;
        }
        if merged.get(key) != base.get(key) && merged.get(key) != Some(value) {
            return Err(format!("SETTINGS_CONFLICT: {key} changed since this form was loaded; reload settings before retrying"));
        }
        merged[key] = value.clone();
    }
    serde_json::from_value(merged).map_err(|e| e.to_string())
}

#[cfg(test)]
mod repository_tests {
    use super::*;
    #[test]
    fn disjoint_patches_preserve_concurrent_changes() {
        let base = AppSettings::default();
        let mut current = base.clone();
        current.antivirus_exclusion_done = true;
        let mut requested = base.clone();
        requested.custom_game_name = "test".into();
        let next = merge_settings(&base, &requested, &current).unwrap();
        assert!(next.antivirus_exclusion_done);
        assert_eq!(next.custom_game_name, "test");
    }
    #[test]
    fn conflicting_patch_is_rejected_without_secret_values() {
        let base = AppSettings::default();
        let mut current = base.clone();
        current.hubcap_api_key = "secret-current".into();
        let mut requested = base.clone();
        requested.hubcap_api_key = "secret-new".into();
        let error = merge_settings(&base, &requested, &current).unwrap_err();
        assert!(error.contains("SETTINGS_CONFLICT"));
        assert!(!error.contains("secret"));
        assert!(merge_settings(&base, &requested, &requested).is_ok());
    }
    #[test]
    fn corrupt_settings_blocks_mutation() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), b"bad-json").unwrap();
        let manager = SettingsManager {
            config_dir: dir.path().into(),
            legacy_config_dir: None,
        };
        assert!(manager
            .update("test", |s| {
                s.custom_game_name = "x".into();
                Ok(())
            })
            .is_err());
        assert_eq!(
            fs::read(dir.path().join("settings.json")).unwrap(),
            b"bad-json"
        );
    }
    #[test]
    fn pending_transaction_replays_both_files() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SettingsManager {
            config_dir: dir.path().into(),
            legacy_config_dir: None,
        };
        // Recovery concerns opaque ciphertext; DPAPI is tested separately on Windows.
        let pending = PendingSettings {
            format_version: 1,
            settings: serde_json::to_string(&AppSettings::default()).unwrap(),
            credentials: vec![1, 2, 3],
        };
        fs::write(
            manager.pending_path(),
            serde_json::to_vec(&pending).unwrap(),
        )
        .unwrap();
        manager.recover_locked().unwrap();
        assert_eq!(fs::read(manager.credentials_path()).unwrap(), vec![1, 2, 3]);
        assert_eq!(
            fs::read_to_string(manager.get_file_path()).unwrap(),
            pending.settings
        );
        assert!(!manager.pending_path().exists());
        manager.recover_locked().unwrap();
    }
    #[test]
    fn failed_second_write_keeps_recovery_record_then_replays() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SettingsManager {
            config_dir: dir.path().into(),
            legacy_config_dir: None,
        };
        let pending = PendingSettings {
            format_version: 1,
            settings: serde_json::to_string(&AppSettings::default()).unwrap(),
            credentials: vec![4, 5, 6],
        };
        fs::write(
            manager.pending_path(),
            serde_json::to_vec(&pending).unwrap(),
        )
        .unwrap();
        fs::create_dir(manager.get_file_path()).unwrap(); // Deterministic commit failure, including as admin.
        assert!(manager.recover_locked().is_err());
        assert!(manager.pending_path().is_file());
        assert_eq!(fs::read(manager.credentials_path()).unwrap(), vec![4, 5, 6]);
        fs::remove_dir(manager.get_file_path()).unwrap();
        manager.recover_locked().unwrap();
        assert!(!manager.pending_path().exists());
        assert_eq!(
            fs::read_to_string(manager.get_file_path()).unwrap(),
            pending.settings
        );
    }
    #[test]
    fn corrupt_journal_never_changes_destinations() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SettingsManager {
            config_dir: dir.path().into(),
            legacy_config_dir: None,
        };
        fs::write(manager.pending_path(), b"bad").unwrap();
        fs::write(manager.get_file_path(), b"original").unwrap();
        assert!(manager.recover_locked().is_err());
        assert_eq!(fs::read(manager.get_file_path()).unwrap(), b"original");
        assert!(!manager.credentials_path().exists());
    }
    #[cfg(windows)]
    #[test]
    fn dpapi_repository_roundtrip_and_concurrent_typed_patches() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SettingsManager {
            config_dir: dir.path().into(),
            legacy_config_dir: None,
        };
        std::thread::scope(|scope| {
            let m = &manager;
            scope.spawn(move || {
                m.update("test-a", |s| {
                    s.hubcap_api_key = "test-secret-only".into();
                    Ok(())
                })
                .unwrap()
            });
            let m = &manager;
            scope.spawn(move || {
                m.update("test-b", |s| {
                    s.antivirus_exclusion_done = true;
                    Ok(())
                })
                .unwrap()
            });
        });
        let loaded = manager.try_load().unwrap();
        assert!(loaded.antivirus_exclusion_done);
        assert_eq!(loaded.hubcap_api_key, "test-secret-only");
        assert!(!fs::read_to_string(manager.get_file_path())
            .unwrap()
            .contains("test-secret-only"));
        assert!(
            !fs::read_to_string(dir.path().join("settings.last-good.json"))
                .unwrap()
                .contains("test-secret-only")
        );
    }
}
