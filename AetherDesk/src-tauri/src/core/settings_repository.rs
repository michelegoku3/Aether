//! Settings repository: `SettingsManager` transactions (atomic journal +
//! recovery), typed merge, health reporting and the `load_settings` /
//! `require_steam_path` conveniences. The data model lives in the sibling
//! `settings_model` module.
use super::settings_model::{
    normalize_icon_selected_file, normalize_library_install_filter, normalize_store_currency,
    normalize_store_front_filter, AppSettings, SettingsLoadFlags,
};
use crate::core::error::{codes, DeskError};
use crate::core::paths::LocalAppPaths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

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
        let _file_lock = crate::core::state_io::lock(&self.config_dir.join("settings.lock"))?;
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
            legacy_config_dir: None,
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
    fn empty_json_matches_default() {
        // Guards ARC-002: every field's serde default must agree with
        // `impl Default`, or upgraders would see different values than
        // fresh installs. `default_steam_path` is detection-based but
        // deterministic within one machine, so the comparison holds here.
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
