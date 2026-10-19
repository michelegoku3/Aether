//! Settings model: `AppSettings`, load-health flags, serde defaults,
//! normalization and currency helpers. Pure data + string logic, no I/O and
//! no Tauri types, so it stays unit-testable in isolation. Persistence lives
//! in the sibling `settings_repository` module.
use serde::{Deserialize, Serialize};

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
    /// Cosmetic Steam library banner: write `AppState.LastOwner` for the
    /// appids in `library_banner_appids`, so the app details page shows
    /// "From the library of <library_banner_steamid>". Purely visual; the DLL
    /// applies it on the next Steam start. All other managed apps keep the
    /// owner entry removed.
    #[serde(default)]
    pub library_banner_enabled: bool,
    /// Appids that opt into the banner (unique, ascending).
    #[serde(default)]
    pub library_banner_appids: Vec<u32>,
    /// Decimal SteamID64 shown as the library owner. Empty = disabled.
    #[serde(default)]
    pub library_banner_steamid: String,
}

/// Health of a settings load, for UI banners. Serialized camelCase for IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsLoadFlags {
    /// Nothing on disk yet (first run): defaults are expected, not alarming.
    pub fresh: bool,
    /// A stored state existed but could not be read: safe defaults are in
    /// use and the UI should say so instead of silently masking it.
    pub corrupt_fallback: bool,
}

/// Single source of truth for plain defaults: serde providers below and
/// `impl Default` both read these, so fresh installs and old settings.json
/// files can never disagree (see ARC-002).
const DEFAULT_ALT_CARDS_OPACITY: u8 = 100;
const DEFAULT_ALT_CARDS_FADE: u8 = 20;
const DEFAULT_STORE_CURRENCY: &str = "eur";
const DEFAULT_STORE_FRONT_FILTER: &str = "upcoming";
const DEFAULT_WALLPAPER_OPACITY: u8 = 20;
const DEFAULT_LIBRARY_INSTALL_FILTER: &str = "all";

/// Serde default provider for the alt-cards backdrop opacity.
fn default_alt_cards_opacity() -> u8 {
    DEFAULT_ALT_CARDS_OPACITY
}

/// Serde default provider for the alt-cards bottom fade.
fn default_alt_cards_fade() -> u8 {
    DEFAULT_ALT_CARDS_FADE
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
    DEFAULT_STORE_CURRENCY.to_string()
}

fn default_store_front_filter() -> String {
    DEFAULT_STORE_FRONT_FILTER.to_string()
}

fn default_wallpaper_opacity() -> u8 {
    DEFAULT_WALLPAPER_OPACITY
}

fn default_library_install_filter() -> String {
    DEFAULT_LIBRARY_INSTALL_FILTER.to_string()
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
            library_banner_enabled: false,
            library_banner_appids: Vec::new(),
            library_banner_steamid: String::new(),
        }
    }
}
