//! Settings repository: `SettingsManager` transactions (atomic journal +
//! recovery), typed merge, health reporting and the `load_settings` /
//! `require_steam_path` conveniences. The data model lives in the sibling
//! `settings_model` module.
use super::settings_model::{
    normalize_store_currency, normalize_store_front_filter, AppSettings, SettingsLoadFlags,
};
use crate::core::error::{codes, DeskError};
use crate::core::paths::LocalAppPaths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub struct SettingsManager {
    config_dir: PathBuf,
}

/// On-disk identity of one settings file. A hand edit is detected by a change
/// in size or modification time; the single-instance guard removes the other
/// writer (a second Desk process), so mtime granularity is the only residual
/// risk and only for same-size writes inside one timestamp tick.
#[derive(Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    exists: bool,
    modified: Option<std::time::SystemTime>,
    len: u64,
}

fn file_stamp(path: &Path) -> FileStamp {
    match fs::metadata(path) {
        Ok(metadata) => FileStamp {
            exists: true,
            modified: metadata.modified().ok(),
            len: metadata.len(),
        },
        Err(_) => FileStamp {
            exists: false,
            modified: None,
            len: 0,
        },
    }
}

fn cache_key(config_dir: &Path) -> (FileStamp, FileStamp) {
    (
        file_stamp(&config_dir.join("settings.json")),
        file_stamp(&config_dir.join("provider_credentials.dat")),
    )
}

struct CachedSettings {
    settings: AppSettings,
    settings_stamp: FileStamp,
    credentials_stamp: FileStamp,
}

fn settings_cache() -> &'static std::sync::RwLock<std::collections::HashMap<PathBuf, CachedSettings>>
{
    static CACHE: std::sync::OnceLock<
        std::sync::RwLock<std::collections::HashMap<PathBuf, CachedSettings>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()))
}

/// Returns the cached snapshot when the files still match it. A pending
/// recovery record disables the cache: a commit was interrupted and the next
/// read must replay it instead of serving the pre-commit bytes.
fn cached_settings(config_dir: &Path) -> Option<AppSettings> {
    if config_dir.join("settings.pending.json").exists() {
        return None;
    }
    let (settings_stamp, credentials_stamp) = cache_key(config_dir);
    let cache = settings_cache().read().ok()?;
    let cached = cache.get(config_dir)?;
    if cached.settings_stamp == settings_stamp && cached.credentials_stamp == credentials_stamp {
        Some(cached.settings.clone())
    } else {
        None
    }
}

fn store_cached_settings(config_dir: &Path, settings: &AppSettings) {
    let (settings_stamp, credentials_stamp) = cache_key(config_dir);
    if let Ok(mut cache) = settings_cache().write() {
        cache.insert(
            config_dir.to_path_buf(),
            CachedSettings {
                settings: settings.clone(),
                settings_stamp,
                credentials_stamp,
            },
        );
    }
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
        Self {
            config_dir: LocalAppPaths::data_root_for_app(app_handle).join("config"),
        }
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
        crate::core::state_io::write_atomic(&self.credentials_path(), &pending.credentials)?;
        crate::core::state_io::write_atomic(&self.get_file_path(), pending.settings.as_bytes())?;
        fs::remove_file(self.pending_path())
            .map_err(|e| format!("Settings committed but recovery cleanup failed: {e}"))?;
        crate::desk_log_info!(
            "settings",
            "Settings transaction finalized (JSON + protected credentials)"
        );
        Ok(())
    }

    fn load_locked(&self) -> Result<AppSettings, String> {
        #[cfg(test)]
        if let Ok(mut counts) = load_counts().lock() {
            *counts.entry(self.config_dir.clone()).or_insert(0) += 1;
        }
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
        // Hit path: no global lock, no file lock, no DPAPI decrypt. Every Desk
        // command reads settings; serializing those reads was the contention.
        if let Some(settings) = cached_settings(&self.config_dir) {
            return Ok(settings);
        }
        let _guard = SETTINGS_LOCK
            .lock()
            .map_err(|_| "Settings repository unavailable")?;
        let _file_lock = crate::core::state_io::lock(&self.config_dir.join("settings.lock"))?;
        let result = self.load_locked();
        if let Ok(settings) = &result {
            store_cached_settings(&self.config_dir, settings);
        }
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

    /// Load with a health report: distinguishes a fresh install (nothing on
    /// disk yet) from a corrupt state that fell back to safe defaults, so
    /// the UI can banner the latter instead of silently masking it.
    pub fn load_reported(&self) -> (AppSettings, SettingsLoadFlags) {
        let outcome = self.try_load();
        let any_file = self.get_file_path().exists() || self.credentials_path().exists();
        match outcome {
            Ok(settings) => (
                settings,
                SettingsLoadFlags {
                    fresh: !any_file,
                    corrupt_fallback: false,
                },
            ),
            Err(_) => (
                AppSettings::default(),
                SettingsLoadFlags {
                    fresh: false,
                    corrupt_fallback: true,
                },
            ),
        }
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
        let _file_lock = crate::core::state_io::lock(&self.config_dir.join("settings.lock"))?;
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
        if let Ok(settings) = &result {
            store_cached_settings(&self.config_dir, settings);
        }
        if let Err(e) = &result {
            crate::desk_log_error!("settings", "Mutation failed operation={}: {}", operation, e);
        }
        result
    }

    /// Like `update`, but when the stored state is unreadable the corrupt
    /// files are quarantined aside and the mutation starts from defaults
    /// instead of failing.
    ///
    /// Returns `(settings, recovered)`. For flows with no meaningful base to
    /// protect (saving over garbage, reset-to-defaults, one-flag
    /// acknowledgements): without this, a single corrupt `settings.json`
    /// dead-ends every mutation, including the save that would fix it.
    /// Lock failures still error — another process may be mid-commit.
    pub fn update_or_reset(
        &self,
        operation: &str,
        change: impl FnOnce(&mut AppSettings) -> Result<(), String>,
    ) -> Result<(AppSettings, bool), String> {
        let _guard = SETTINGS_LOCK
            .lock()
            .map_err(|_| "Settings repository unavailable")?;
        let _file_lock = crate::core::state_io::lock(&self.config_dir.join("settings.lock"))?;
        let (previous, recovered) = match self.load_locked() {
            Ok(previous) => (previous, false),
            Err(error) => {
                crate::desk_log_warn!(
                    "settings",
                    "Stored state unreadable ({}); quarantining aside and starting from defaults",
                    error
                );
                self.quarantine_corrupt()?;
                (AppSettings::default(), true)
            }
        };
        let mut next = previous.clone();
        change(&mut next)?;
        normalize_settings(&mut next);
        if next == previous && !recovered {
            crate::desk_log_debug!("settings", "Mutation unchanged operation={}", operation);
            return Ok((next, false));
        }
        self.commit_locked(&previous, &next)?;
        crate::desk_log_info!(
            "settings",
            "Mutation committed operation={} recovered={} (values redacted)",
            operation,
            recovered
        );
        store_cached_settings(&self.config_dir, &next);
        Ok((next, recovered))
    }

    /// Rename unreadable state files aside (never delete: the bytes may still
    /// hold the user's keys). Best-effort per file; a failed rename surfaces
    /// because proceeding would silently orphan the data.
    fn quarantine_corrupt(&self) -> Result<(), String> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        for name in [
            "settings.json",
            "provider_credentials.dat",
            "settings.pending.json",
        ] {
            let from = self.config_dir.join(name);
            if !from.exists() {
                continue;
            }
            let to = self.config_dir.join(format!("{name}.corrupt-{stamp}.bak"));
            fs::rename(&from, &to).map_err(|e| format!("Cannot quarantine corrupt {name}: {e}"))?;
        }
        Ok(())
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
            crate::core::state_io::write_atomic(
                &self.config_dir.join("settings.last-good.json"),
                &serde_json::to_vec(&backup).map_err(|e| e.to_string())?,
            )?;
        }
        crate::core::state_io::write_atomic(
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

/// Convenience: load settings in one call. The cache check comes before the
/// manager so a hit skips the migration probe, the file lock and DPAPI.
#[inline]
pub fn load_settings(app: &tauri::AppHandle) -> AppSettings {
    let config_dir = LocalAppPaths::data_root_for_app(app).join("config");
    if let Some(settings) = cached_settings(&config_dir) {
        return settings;
    }
    SettingsManager::new(app).load()
}

/// Convenience: return a non-empty `steam_path` or a descriptive error ready
/// for `?` propagation. Centralises the "steam path required" guard.
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

/// Test-only observation of how many times a config dir's files were actually
/// read. Keyed by directory so parallel tests in other temp dirs cannot skew
/// the count.
#[cfg(test)]
fn load_counts() -> &'static std::sync::Mutex<std::collections::HashMap<PathBuf, usize>> {
    static COUNTS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<PathBuf, usize>>,
    > = std::sync::OnceLock::new();
    COUNTS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
fn load_count(dir: &Path) -> usize {
    load_counts()
        .lock()
        .map(|counts| counts.get(dir).copied().unwrap_or(0))
        .unwrap_or(0)
}
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
    settings.personal_wallpaper_opacity = settings.personal_wallpaper_opacity.min(100);
    settings.alternative_cards_opacity = settings.alternative_cards_opacity.min(100);
    settings.alternative_cards_fade = settings.alternative_cards_fade.min(100);
    settings.library_banner_appids.sort_unstable();
    settings.library_banner_appids.dedup();
    // A SteamID64 is a decimal unsigned 64-bit id. Parsing (not only checking
    // the digits) also rejects values above 2^64-1, so a hand-edited settings
    // file cannot carry one: the banner policy requires a valid account.
    let steamid_valid = settings.library_banner_steamid.is_empty()
        || (settings
            .library_banner_steamid
            .bytes()
            .all(|b| b.is_ascii_digit())
            && settings.library_banner_steamid.parse::<u64>().is_ok());
    if !steamid_valid {
        settings.library_banner_steamid.clear();
    }
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
            return Err(DeskError::coded(
                codes::SETTINGS_CONFLICT,
                format_args!(
                    "{key} changed since this form was loaded; reload settings before retrying"
                ),
            ));
        }
        merged[key] = value.clone();
    }
    serde_json::from_value(merged).map_err(|e| e.to_string())
}

#[cfg(test)]
mod repository_tests {
    use super::*;
    #[test]
    fn load_reported_distinguishes_fresh_loaded_and_corrupt() {
        // Fresh install: nothing on disk.
        let dir = tempfile::tempdir().unwrap();
        let manager = SettingsManager {
            config_dir: dir.path().into(),
        };
        let (settings, flags) = manager.load_reported();
        assert_eq!(settings, AppSettings::default());
        assert!(flags.fresh && !flags.corrupt_fallback);

        // Healthy stored state.
        manager
            .update("test", |s| {
                s.custom_game_name = "kept".into();
                Ok(())
            })
            .unwrap();
        let (settings, flags) = manager.load_reported();
        assert_eq!(settings.custom_game_name, "kept");
        assert!(!flags.fresh && !flags.corrupt_fallback);

        // Corrupt JSON: safe defaults + banner flag, files untouched.
        fs::write(dir.path().join("settings.json"), b"bad-json").unwrap();
        let (settings, flags) = manager.load_reported();
        assert_eq!(settings, AppSettings::default());
        assert!(!flags.fresh && flags.corrupt_fallback);
        assert_eq!(
            fs::read(dir.path().join("settings.json")).unwrap(),
            b"bad-json"
        );
    }

    #[test]
    fn repeated_loads_reuse_the_cached_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let stored = AppSettings {
            custom_game_name: "cold".into(),
            ..AppSettings::default()
        };
        fs::write(
            dir.path().join("settings.json"),
            serde_json::to_vec(&stored).unwrap(),
        )
        .unwrap();
        let manager = SettingsManager {
            config_dir: dir.path().into(),
        };

        let before = load_count(dir.path());
        assert_eq!(manager.try_load().unwrap().custom_game_name, "cold");
        assert_eq!(manager.try_load().unwrap().custom_game_name, "cold");

        assert_eq!(
            load_count(dir.path()) - before,
            1,
            "the second load must be served from the cache"
        );
    }

    #[test]
    fn an_external_file_change_invalidates_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SettingsManager {
            config_dir: dir.path().into(),
        };
        let path = dir.path().join("settings.json");
        let stored = AppSettings {
            custom_game_name: "before".into(),
            ..AppSettings::default()
        };
        fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        assert_eq!(manager.try_load().unwrap().custom_game_name, "before");

        // Different size, so the stamp changes even if the mtime tick is the
        // same: a hand edit must never be hidden by the cache.
        let mut edited = stored;
        edited.custom_game_name = "an external edit with a different size".into();
        fs::write(&path, serde_json::to_vec(&edited).unwrap()).unwrap();

        assert_eq!(
            manager.try_load().unwrap().custom_game_name,
            "an external edit with a different size"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_pending_transaction_is_not_masked_by_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SettingsManager {
            config_dir: dir.path().into(),
        };
        let stored = AppSettings {
            custom_game_name: "before".into(),
            ..AppSettings::default()
        };
        fs::write(
            manager.get_file_path(),
            serde_json::to_vec(&stored).unwrap(),
        )
        .unwrap();
        assert_eq!(manager.try_load().unwrap().custom_game_name, "before");

        let recovered = AppSettings {
            custom_game_name: "recovered".into(),
            ..AppSettings::default()
        };
        let pending = PendingSettings {
            format_version: 1,
            settings: serde_json::to_string(&recovered).unwrap(),
            credentials: crate::core::secure_storage::protect(
                br#"{"hubcap_api_key":"","ryuu_api_key":""}"#,
            )
            .unwrap(),
        };
        fs::write(
            manager.pending_path(),
            serde_json::to_vec(&pending).unwrap(),
        )
        .unwrap();

        assert_eq!(manager.try_load().unwrap().custom_game_name, "recovered");
        assert!(!manager.pending_path().exists());
    }
    #[test]
    fn empty_json_matches_default() {
        // Every field's serde default must agree with `impl Default`, or
        // upgraders would see different values than fresh installs.
        // `default_steam_path` is detection-based but deterministic within one
        // machine, so the comparison holds here.
        let from_empty: AppSettings = serde_json::from_str("{}").expect("empty object must parse");
        assert_eq!(from_empty, AppSettings::default());
    }
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
    fn normalize_settings_bounds_the_library_owner_id() {
        let mut settings = AppSettings {
            library_banner_steamid: "18446744073709551615".into(), // 2^64 - 1
            ..AppSettings::default()
        };
        normalize_settings(&mut settings);
        assert_eq!(settings.library_banner_steamid, "18446744073709551615");

        settings.library_banner_steamid = "18446744073709551616".into(); // 2^64
        normalize_settings(&mut settings);
        assert!(settings.library_banner_steamid.is_empty());

        settings.library_banner_steamid = "76561199876393402x".into();
        normalize_settings(&mut settings);
        assert!(settings.library_banner_steamid.is_empty());
    }
    #[test]
    fn update_or_reset_quarantines_garbage_and_starts_from_defaults() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), b"bad-json").unwrap();
        let manager = SettingsManager {
            config_dir: dir.path().into(),
        };
        let (saved, recovered) = manager
            .update_or_reset("test", |s| {
                s.custom_game_name = "fresh start".into();
                Ok(())
            })
            .unwrap();
        assert!(recovered);
        assert_eq!(saved.custom_game_name, "fresh start");
        // The garbage is preserved aside, never deleted; the live file is valid.
        let quarantined: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok().map(|e| e.file_name()))
            .filter(|name| name.to_string_lossy().contains("corrupt-"))
            .collect();
        assert_eq!(quarantined.len(), 1);
        let loaded = manager.try_load().unwrap();
        assert_eq!(loaded.custom_game_name, "fresh start");
    }
    #[test]
    fn update_or_reset_keeps_healthy_state_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SettingsManager {
            config_dir: dir.path().into(),
        };
        manager
            .update("test", |s| {
                s.custom_game_name = "kept".into();
                Ok(())
            })
            .unwrap();
        let (saved, recovered) = manager
            .update_or_reset("test", |s| {
                s.steam_path = "C:\\Games\\Steam".into();
                Ok(())
            })
            .unwrap();
        assert!(!recovered);
        assert_eq!(saved.custom_game_name, "kept");
        assert_eq!(saved.steam_path, "C:\\Games\\Steam");
        let stray: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok().map(|e| e.file_name()))
            .filter(|name| name.to_string_lossy().contains("corrupt-"))
            .collect();
        assert!(stray.is_empty());
    }
    #[test]
    fn corrupt_settings_blocks_mutation() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), b"bad-json").unwrap();
        let manager = SettingsManager {
            config_dir: dir.path().into(),
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
