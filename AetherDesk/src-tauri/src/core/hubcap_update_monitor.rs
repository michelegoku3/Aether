//! Event-driven monitor for authenticated Hubcap game package updates.
//!
//! Steam is the change source: an `appmanifest_<appid>.acf` fingerprint changes
//! when Steam has actually completed a build update. Only then is the
//! authenticated Hubcap package endpoint called. This deliberately does not
//! poll Hubcap and never walks every Lua file looking for work.
//!
//! The monitor is local-first and crash-resumable:
//! - the first run records the current Steam state without downloading anything;
//! - a changed ACF is queued only for a managed game whose pins allow updates;
//! - one package is processed at a time (the provider has account/rate limits);
//! - the last successfully processed fingerprint is persisted atomically;
//! - failures remain queued with bounded exponential backoff.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::AppHandle;

use crate::core::paths::LocalAppPaths;
use crate::core::settings::SettingsManager;
use crate::manifest::pins::LuaManifestPins;
use crate::steam::library::SteamLibraryScanner;

const START_DELAY: Duration = Duration::from_secs(8);
const POLL_INTERVAL: Duration = Duration::from_secs(20);
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30 * 60);
const STATE_FILE: &str = "hubcap_game_updates.json";

#[derive(Debug, Clone, Default)]
struct AppManifestSnapshot {
    /// One aggregate fingerprint per AppID. An app can be present in more than
    /// one library, so the paths are sorted before aggregation.
    by_app: HashMap<u32, String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct PersistedState {
    initialized: bool,
    /// AppID -> last Steam appmanifest fingerprint for which a Hubcap package
    /// completed successfully. This is not a migration marker: it is the
    /// resumable updater checkpoint.
    processed: HashMap<u32, String>,
    #[serde(default)]
    lua_processed: HashMap<u32, String>,
    #[serde(default)]
    workshop_processed: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingUpdate {
    attempts: u32,
    next_attempt: Instant,
}

fn state_path() -> PathBuf {
    LocalAppPaths::state_dir().join(STATE_FILE)
}

fn load_state() -> PersistedState {
    let path = state_path();
    let Ok(bytes) = fs::read(&path) else {
        return PersistedState::default();
    };
    match serde_json::from_slice(&bytes) {
        Ok(state) => state,
        Err(error) => {
            crate::desk_log_warn!(
                "hubcap-updates",
                "Ignoring invalid updater checkpoint at {}: {}",
                path.display(),
                error
            );
            PersistedState::default()
        }
    }
}

fn save_state(state: &PersistedState) -> Result<(), String> {
    let path = state_path();
    let parent = path
        .parent()
        .ok_or_else(|| "Hubcap updater state has no parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create Hubcap updater state directory: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("Could not serialize Hubcap updater state: {error}"))?;
    fs::write(&temporary, bytes)
        .map_err(|error| format!("Could not write Hubcap updater state: {error}"))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("Could not commit Hubcap updater state: {error}"))
}

fn app_id_from_manifest(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    name.strip_prefix("appmanifest_")?
        .strip_suffix(".acf")?
        .parse::<u32>()
        .ok()
        .filter(|id| *id > 0)
}

fn file_fingerprint(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let digest = Sha256::digest(bytes);
    Some(format!("{digest:x}"))
}

fn scan_app_manifests(steam_path: &str) -> AppManifestSnapshot {
    let scanner = SteamLibraryScanner::new(steam_path);
    let mut per_app: HashMap<u32, Vec<(String, String)>> = HashMap::new();

    for library in scanner.discover_library_paths() {
        let steamapps = library.join("steamapps");
        let Ok(entries) = fs::read_dir(steamapps) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(app_id) = app_id_from_manifest(&path) else {
                continue;
            };
            let Some(fingerprint) = file_fingerprint(&path) else {
                continue;
            };
            per_app
                .entry(app_id)
                .or_default()
                .push((path.to_string_lossy().into_owned(), fingerprint));
        }
    }

    let mut by_app = HashMap::new();
    for (app_id, mut files) in per_app {
        files.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        let mut hasher = Sha256::new();
        for (path, fingerprint) in files {
            hasher.update(path.as_bytes());
            hasher.update([0]);
            hasher.update(fingerprint.as_bytes());
            hasher.update([0]);
        }
        by_app.insert(app_id, format!("{:x}", hasher.finalize()));
    }
    AppManifestSnapshot { by_app }
}

fn scan_managed_luas(steam_path: &str) -> HashMap<u32, String> {
    let directory = PathBuf::from(steam_path).join("config").join("stplug-in");
    let Ok(entries) = fs::read_dir(directory) else {
        return HashMap::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let app_id = path
                .extension()
                .and_then(|extension| extension.to_str())
                .filter(|extension| extension.eq_ignore_ascii_case("lua"))
                .and_then(|_| path.file_stem())
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse::<u32>().ok())
                .filter(|id| *id > 0)?;
            Some((app_id, file_fingerprint(&path)?))
        })
        .collect()
}

fn scan_workshop_manifests(steam_path: &str) -> Option<String> {
    let scanner = SteamLibraryScanner::new(steam_path);
    let mut files = Vec::new();
    for library in scanner.discover_library_paths() {
        let directory = library.join("steamapps").join("workshop");
        let Ok(entries) = fs::read_dir(directory) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_acf = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("appworkshop_") && name.ends_with(".acf"))
                .unwrap_or(false);
            if !is_acf { continue; }
            if let Some(fingerprint) = file_fingerprint(&path) {
                files.push((path.to_string_lossy().into_owned(), fingerprint));
            }
        }
    }
    if files.is_empty() {
        return None;
    }
    files.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = Sha256::new();
    for (path, fingerprint) in files {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(fingerprint.as_bytes());
        hasher.update([0]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

fn managed_update_candidates(
    changed_apps: impl IntoIterator<Item = u32>,
    steam_path: &str,
) -> Vec<u32> {
    let mut candidates = Vec::new();
    for app_id in changed_apps {
        let pins = LuaManifestPins::new(steam_path.to_string(), app_id);
        if !pins.path_exists() {
            continue;
        }
        match pins.updates_are_enabled() {
            Ok(true) => candidates.push(app_id),
            Ok(false) => crate::desk_log_debug!(
                "hubcap-updates",
                "Ignoring Steam change app_id={} because its Lua pins are still enabled for a fixed version",
                app_id
            ),
            Err(error) => crate::desk_log_warn!(
                "hubcap-updates",
                "Could not inspect update policy app_id={}: {}",
                app_id,
                error
            ),
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates
}

fn retry_delay(attempts: u32) -> Duration {
    let multiplier = 1u64 << attempts.min(6);
    INITIAL_RETRY_DELAY
        .checked_mul(multiplier as u32)
        .unwrap_or(MAX_RETRY_DELAY)
        .min(MAX_RETRY_DELAY)
}

fn changed_app_ids(previous: &AppManifestSnapshot, current: &AppManifestSnapshot) -> Vec<u32> {
    current
        .by_app
        .iter()
        .filter_map(|(app_id, fingerprint)| {
            (previous.by_app.get(app_id) != Some(fingerprint)).then_some(*app_id)
        })
        .collect()
}

async fn run(app: AppHandle) {
    tokio::time::sleep(START_DELAY).await;
    crate::desk_log_info!(
        "hubcap-updates",
        "Authenticated Hubcap game update monitor started (ACF poll every {:?}, max concurrency=1)",
        POLL_INTERVAL
    );

    let mut checkpoint = load_state();
    let mut previous = AppManifestSnapshot::default();
    let mut pending: BTreeMap<u32, PendingUpdate> = BTreeMap::new();
    let mut manifest_repairs: BTreeMap<u32, PendingUpdate> = BTreeMap::new();
    let mut workshop_pending: Option<PendingUpdate> = None;
    let mut first_scan = true;

    loop {
        let settings = SettingsManager::new(&app).load();
        let current = if settings.steam_path.trim().is_empty() {
            AppManifestSnapshot::default()
        } else {
            scan_app_manifests(&settings.steam_path)
        };
        let current_luas = if settings.steam_path.trim().is_empty() {
            HashMap::new()
        } else {
            scan_managed_luas(&settings.steam_path)
        };
        let current_workshop = if settings.steam_path.trim().is_empty() {
            None
        } else {
            scan_workshop_manifests(&settings.steam_path)
        };

        if first_scan {
            if !checkpoint.initialized {
                // Do not turn an existing library into a quota-consuming first
                // sync. The current fingerprints become the baseline; future
                // Steam changes, or a failed in-flight operation, create work.
                checkpoint.initialized = true;
                checkpoint.processed = current.by_app.clone();
                checkpoint.lua_processed = current_luas.clone();
                checkpoint.workshop_processed = current_workshop.clone();
                if let Err(error) = save_state(&checkpoint) {
                    crate::desk_log_warn!("hubcap-updates", "Could not initialize updater checkpoint: {}", error);
                }
            } else {
                let changed = current
                    .by_app
                    .iter()
                    .filter_map(|(app_id, fingerprint)| {
                        (checkpoint.processed.get(app_id) != Some(fingerprint)).then_some(*app_id)
                    });
                for app_id in managed_update_candidates(changed, &settings.steam_path) {
                    pending.entry(app_id).or_insert(PendingUpdate {
                        attempts: 0,
                        next_attempt: Instant::now(),
                    });
                }
                if current_workshop != checkpoint.workshop_processed {
                    if current_workshop.is_some() {
                        workshop_pending = Some(PendingUpdate {
                            attempts: 0,
                            next_attempt: Instant::now(),
                        });
                    } else {
                        checkpoint.workshop_processed = None;
                        let _ = save_state(&checkpoint);
                    }
                }
            }
            previous = current.clone();
            first_scan = false;
        } else {
            let mut changed = changed_app_ids(&previous, &current);
            previous = current.clone();
            if settings.download_games_with_updates_on
                && !settings.hubcap_api_key.trim().is_empty()
                && !settings.steam_path.trim().is_empty()
            {
                // Also inspect the durable checkpoint. This catches a Steam
                // update that happened while the feature/key was disabled and
                // lets an interrupted operation resume without a new ACF write.
                changed.extend(current.by_app.iter().filter_map(|(app_id, fingerprint)| {
                    (checkpoint.processed.get(app_id) != Some(fingerprint)).then_some(*app_id)
                }));
                for app_id in managed_update_candidates(changed, &settings.steam_path) {
                    pending.entry(app_id).or_insert(PendingUpdate {
                        attempts: 0,
                        next_attempt: Instant::now(),
                    });
                }
            }
        }

        // A local Lua replacement is a manifest-consistency event, not a
        // request to enable automatic latest-game updates. Repair it whenever
        // authenticated Hubcap is configured, even if the latest-version
        // policy toggle is off.
        if !settings.hubcap_api_key.trim().is_empty()
            && !settings.steam_path.trim().is_empty()
        {
            let changed_luas = current_luas.iter().filter_map(|(app_id, fingerprint)| {
                (checkpoint.lua_processed.get(app_id) != Some(fingerprint)).then_some(*app_id)
            });
            for app_id in changed_luas {
                manifest_repairs.entry(app_id).or_insert(PendingUpdate {
                    attempts: 0,
                    next_attempt: Instant::now(),
                });
            }
        }

        if !first_scan && current_workshop != checkpoint.workshop_processed {
            if current_workshop.is_some() {
                if workshop_pending.is_none() {
                    workshop_pending = Some(PendingUpdate {
                        attempts: 0,
                        next_attempt: Instant::now(),
                    });
                }
            } else {
                checkpoint.workshop_processed = None;
                let _ = save_state(&checkpoint);
            }
        }

        if settings.download_games_with_updates_on
            && !settings.hubcap_api_key.trim().is_empty()
            && !pending.is_empty()
        {
            let now = Instant::now();
            let ready = pending
                .iter()
                .find(|(_, update)| update.next_attempt <= now)
                .map(|(app_id, _)| *app_id);
            if let Some(app_id) = ready {
                let update = pending.remove(&app_id).expect("ready update exists");
                crate::desk_log_info!(
                    "hubcap-updates",
                    "Processing Steam build change app_id={} attempt={}",
                    app_id,
                    update.attempts + 1
                );
                let result = crate::commands::store::trigger_hubcap_download(
                    app.clone(),
                    app_id,
                    settings.hubcap_api_key.clone(),
                    settings.steam_path.clone(),
                )
                .await;
                match result {
                    Ok(_) => {
                        if let Some(fingerprint) = current.by_app.get(&app_id) {
                            checkpoint.processed.insert(app_id, fingerprint.clone());
                            if let Err(error) = save_state(&checkpoint) {
                                crate::desk_log_warn!(
                                    "hubcap-updates",
                                    "Package installed but updater checkpoint could not be saved app_id={}: {}",
                                    app_id,
                                    error
                                );
                            }
                        }
                        crate::desk_log_info!(
                            "hubcap-updates",
                            "Hubcap package update complete app_id={}",
                            app_id
                        );
                    }
                    Err(error) => {
                        let attempts = update.attempts.saturating_add(1);
                        crate::desk_log_warn!(
                            "hubcap-updates",
                            "Hubcap package update deferred app_id={} attempts={} retry_in_secs={} error={}",
                            app_id,
                            attempts,
                            retry_delay(attempts).as_secs(),
                            error
                        );
                        pending.insert(
                            app_id,
                            PendingUpdate {
                                attempts,
                                next_attempt: Instant::now() + retry_delay(attempts),
                            },
                        );
                    }
                }
            }
        }

        if !settings.hubcap_api_key.trim().is_empty()
            && !manifest_repairs.is_empty()
        {
            let now = Instant::now();
            let ready = manifest_repairs
                .iter()
                .find(|(_, update)| update.next_attempt <= now)
                .map(|(app_id, _)| *app_id);
            if let Some(app_id) = ready {
                let update = manifest_repairs.remove(&app_id).expect("ready repair exists");
                match crate::commands::manifests::sync_hubcap_game_manifest(app.clone(), app_id).await {
                    Ok(_) => {
                        if let Some(fingerprint) = current_luas.get(&app_id) {
                            checkpoint.lua_processed.insert(app_id, fingerprint.clone());
                            if let Err(error) = save_state(&checkpoint) {
                                crate::desk_log_warn!(
                                    "hubcap-updates",
                                    "Manifest repair completed but checkpoint could not be saved app_id={}: {}",
                                    app_id,
                                    error
                                );
                            }
                        }
                        crate::desk_log_info!("hubcap-updates", "Local Lua manifest repair complete app_id={}", app_id);
                    }
                    Err(error) => {
                        let attempts = update.attempts.saturating_add(1);
                        let delay = retry_delay(attempts);
                        crate::desk_log_warn!(
                            "hubcap-updates",
                            "Local Lua manifest repair deferred app_id={} attempts={} retry_in_secs={} error={}",
                            app_id,
                            attempts,
                            delay.as_secs(),
                            error
                        );
                        manifest_repairs.insert(app_id, PendingUpdate {
                            attempts,
                            next_attempt: Instant::now() + delay,
                        });
                    }
                }
            }
        }

        if !settings.hubcap_api_key.trim().is_empty() {
            let ready = workshop_pending
                .as_ref()
                .filter(|update| update.next_attempt <= Instant::now())
                .is_some();
            if ready {
                let update = workshop_pending.take().expect("ready Workshop update exists");
                match crate::commands::workshop::sync_hubcap_workshop_manifests(app.clone()).await {
                    Ok(report) => {
                        crate::desk_log_info!(
                            "hubcap-updates",
                            "Workshop change processed discovered={} generated={} cached={} complete={} content_missing={} failed={}",
                            report.discovered,
                            report.generated,
                            report.restored_from_cache,
                            report.already_local,
                            report.content_missing,
                            report.failed
                        );
                        if report.failed == 0 {
                            checkpoint.workshop_processed = current_workshop.clone();
                            if let Err(error) = save_state(&checkpoint) {
                                crate::desk_log_warn!(
                                    "hubcap-updates",
                                    "Workshop sync completed but checkpoint could not be saved: {}",
                                    error
                                );
                            }
                        } else {
                            let attempts = update.attempts.saturating_add(1);
                            workshop_pending = Some(PendingUpdate {
                                attempts,
                                next_attempt: Instant::now() + retry_delay(attempts),
                            });
                        }
                    }
                    Err(error) => {
                        let attempts = update.attempts.saturating_add(1);
                        let delay = retry_delay(attempts);
                        crate::desk_log_warn!(
                            "hubcap-updates",
                            "Workshop sync deferred attempts={} retry_in_secs={} error={}",
                            attempts,
                            delay.as_secs(),
                            error
                        );
                        workshop_pending = Some(PendingUpdate {
                            attempts,
                            next_attempt: Instant::now() + delay,
                        });
                    }
                }
            }
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Starts the single process-wide monitor. Tauri setup calls this once.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(run(app));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_apps_only_include_new_or_changed_fingerprints() {
        let previous = AppManifestSnapshot {
            by_app: HashMap::from([(10, "old".to_string()), (20, "same".to_string())]),
        };
        let current = AppManifestSnapshot {
            by_app: HashMap::from([
                (10, "new".to_string()),
                (20, "same".to_string()),
                (30, "new".to_string()),
            ]),
        };
        let mut changed = changed_app_ids(&previous, &current);
        changed.sort_unstable();
        assert_eq!(changed, vec![10, 30]);
    }

    #[test]
    fn retry_delay_is_bounded() {
        assert_eq!(retry_delay(0), INITIAL_RETRY_DELAY);
        assert_eq!(retry_delay(99), MAX_RETRY_DELAY);
    }
}
