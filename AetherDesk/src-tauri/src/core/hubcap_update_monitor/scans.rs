//! Steam-side fingerprints and durable synchronizer checkpoint.
//!
//! Owns appmanifest, managed-Lua and Workshop scans plus atomic checkpoint
//! persistence. It has no Tauri dependency: only filesystem and hashing.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::paths::LocalAppPaths;
use crate::manifest::pins::LuaManifestPins;
use crate::steam::library::SteamLibraryScanner;

const STATE_FILE: &str = "hubcap_game_updates.json";

#[derive(Debug, Serialize, Deserialize, Default)]
pub(super) struct PersistedState {
    pub(super) initialized: bool,
    /// AppID -> last Steam ACF fingerprint whose pin sync completed (or that
    /// was inspected and needed no sync). This is the resumable checkpoint,
    /// not a migration marker.
    #[serde(default)]
    pub(super) processed: HashMap<u32, String>,
    #[serde(default)]
    pub(super) lua_processed: HashMap<u32, String>,
    #[serde(default)]
    pub(super) workshop_processed: Option<String>,
    /// AppID -> epoch seconds of the last completed Hubcap-contents pin
    /// refresh check. Throttles the pin_refresh lane (free endpoint, but the
    /// diff must not run on every poll for every game).
    #[serde(default)]
    pub(super) contents_checked: HashMap<u32, u64>,
    /// AppID -> Steam ACF fingerprint observed at the last completed contents
    /// check. Equal to the current fingerprint means "Steam did not touch this
    /// game since", which is what promotes an app from the idle (1 h) to the
    /// fast (15 min) contents cadence.
    #[serde(default)]
    pub(super) contents_fingerprint: HashMap<u32, String>,
}

fn state_path() -> PathBuf {
    LocalAppPaths::state_dir().join(STATE_FILE)
}

pub(super) fn load_state() -> PersistedState {
    let path = state_path();
    let Ok(bytes) = fs::read(&path) else {
        return PersistedState::default();
    };
    match serde_json::from_slice(&bytes) {
        Ok(state) => state,
        Err(error) => {
            crate::desk_log_warn!(
                "hubcap-updates",
                "Ignoring invalid synchronizer checkpoint at {}: {}",
                path.display(),
                error
            );
            PersistedState::default()
        }
    }
}

pub(super) fn save_state(state: &PersistedState) -> Result<(), String> {
    let path = state_path();
    let parent = path
        .parent()
        .ok_or_else(|| "Synchronizer state has no parent directory".to_string())?
        .to_path_buf();
    fs::create_dir_all(&parent)
        .map_err(|error| format!("Could not create synchronizer state directory: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("Could not serialize synchronizer state: {error}"))?;
    fs::write(&temporary, bytes)
        .map_err(|error| format!("Could not write synchronizer state: {error}"))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("Could not commit synchronizer state: {error}"))
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

pub(super) fn scan_app_manifests(steam_path: &str) -> HashMap<u32, String> {
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
        // An app can be present in more than one library: aggregate sorted
        // path+fingerprint pairs so the digest is stable regardless of
        // directory iteration order.
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
    by_app
}

pub(super) fn scan_managed_luas(steam_path: &str) -> HashMap<u32, String> {
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

pub(super) fn scan_workshop_manifests(steam_path: &str) -> Option<String> {
    let scanner = SteamLibraryScanner::new(steam_path);
    let mut files = Vec::new();
    for library in scanner.discover_library_paths() {
        let directory = library.join("steamapps").join("workshop");
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_acf = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("appworkshop_") && name.ends_with(".acf"))
                .unwrap_or(false);
            if !is_acf {
                continue;
            }
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

/// AppIDs whose current fingerprint differs from the checkpoint's.
pub(super) fn changed_against(
    checkpoint: &HashMap<u32, String>,
    current: &HashMap<u32, String>,
) -> Vec<u32> {
    current
        .iter()
        .filter_map(|(app_id, fingerprint)| {
            (checkpoint.get(app_id) != Some(fingerprint)).then_some(*app_id)
        })
        .collect()
}

/// Games whose Lua pins allow updates (at least one commented setManifestid
/// with an active addappid). Everything else is an explicit version lock and
/// must never be realigned.
pub(super) fn managed_update_candidates(
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
                "No pin sync needed app_id={}: its Lua pins are still enabled for a fixed version",
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

