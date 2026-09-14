//! Background synchronizer for Steam-side changes (the Fase-4 redesign of the
//! old update monitor).
//!
//! Steam is the change source: `appmanifest_<appid>.acf`, `stplug-in/*.lua`
//! and `appworkshop_*.acf` fingerprints change only when Steam (or the user)
//! actually did something. The monitor reacts with LOCAL work only — it never
//! downloads game packages and never spends Hubcap quota on its own; manifest
//! generation on demand stays with AetherDLL inside Steam.
//!
//! Four independent lanes share one poll loop:
//! - **pin_sync**: after Steam updated a game whose pins allow updates, the
//!   commented `setManifestid` rows are realigned to the manifests Steam
//!   actually installed and those manifests are archived into the AetherData
//!   backup (so a future uninstall can still restore the current version).
//!   Purely local: no provider traffic.
//! - **pin_refresh**: on a per-app timer (checkpointed), the Lua pins of
//!   updates-ON games are diffed against the manifests Hubcap currently
//!   packages through the FREE `/manifest/{appid}/contents` endpoint (no ZIP
//!   download; the check itself costs no quota). Moved GIDs are staged
//!   locally — generation only for genuinely missing manifests — and the
//!   commented pins are realigned, so "Disable updates" later locks the
//!   CURRENT version instead of a stale GID. Hubcap-only: no SteamDB, no
//!   Depotbox.
//! - **repair**: a changed Lua is a manifest-consistency event, not a request
//!   to change version. It is repaired through the shared local-first
//!   `sync_hubcap_game_manifest` command (provider only for what is missing).
//! - **workshop**: MANUAL ONLY via the Library `Repair Workshop` button
//!   (`sync_hubcap_workshop_manifests`). The automatic lane is disabled:
//!   AetherDLL inside Steam is the default owner of Workshop manifests.
//!
//! Resumability and pacing mirror the original design: the first run records
//! the current Steam state without doing anything, the last processed
//! fingerprint per source is persisted atomically, one task per lane runs per
//! poll, and failures stay queued with bounded exponential backoff — all
//! visible to the UI through `get_hubcap_monitor_status`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::AppHandle;

use crate::core::paths::LocalAppPaths;
use crate::core::settings::SettingsManager;
use crate::manifest::pins::{DepotManifestPin, LuaManifestPins};
use crate::steam::library::SteamLibraryScanner;

const START_DELAY: Duration = Duration::from_secs(8);
const POLL_INTERVAL: Duration = Duration::from_secs(20);
pub(crate) const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(30);
pub(crate) const MAX_RETRY_DELAY: Duration = Duration::from_secs(30 * 60);
const STATE_FILE: &str = "hubcap_game_updates.json";
/// How often the pin_refresh lane may re-check one app against Hubcap's free
/// contents endpoint. The check costs no quota; only genuinely missing
/// manifests are generated, so a moderate interval keeps the commented pins
/// fresh without hammering the provider.
const CONTENTS_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Legacy workshop deferred delay (automatic lane disabled; manual Repair
/// Workshop runs on demand from the Library button instead).
#[allow(dead_code)]
const WORKSHOP_DEFERRED_RETRY_DELAY: Duration = Duration::from_secs(90);

// ============================================================================
// Status snapshot (UI-facing)
// ============================================================================

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PendingTaskInfo {
    /// `None` for lane-wide tasks (the Workshop sync is not per-app).
    pub app_id: Option<u32>,
    pub attempts: u32,
    /// Unix epoch seconds of the next scheduled attempt, when known.
    pub next_retry_epoch: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LaneStatus {
    pub pending: Vec<PendingTaskInfo>,
    /// Tasks completed successfully since the monitor started.
    pub processed_count: u64,
    /// Unix epoch seconds of the last completed task.
    pub last_run_epoch: Option<u64>,
    /// Last failure message, kept until the next success.
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MonitorStatusSnapshot {
    pub running: bool,
    pub started_epoch: Option<u64>,
    pub last_scan_epoch: Option<u64>,
    pub steam_path_configured: bool,
    pub hubcap_key_configured: bool,
    pub checkpoint_initialized: bool,
    pub pin_sync: LaneStatus,
    pub pin_refresh: LaneStatus,
    pub repair: LaneStatus,
    pub workshop: LaneStatus,
}

fn status_store() -> &'static Mutex<MonitorStatusSnapshot> {
    static STORE: OnceLock<Mutex<MonitorStatusSnapshot>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(MonitorStatusSnapshot::default()))
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Reads the live monitor state for the UI (`get_hubcap_monitor_status`).
pub fn snapshot() -> MonitorStatusSnapshot {
    status_store()
        .lock()
        .map(|status| status.clone())
        .unwrap_or_default()
}

fn with_status(update: impl FnOnce(&mut MonitorStatusSnapshot)) {
    if let Ok(mut status) = status_store().lock() {
        update(&mut status);
    }
}

// ============================================================================
// Durable checkpoint
// ============================================================================

#[derive(Debug, Serialize, Deserialize, Default)]
struct PersistedState {
    initialized: bool,
    /// AppID -> last Steam ACF fingerprint whose pin sync completed (or that
    /// was inspected and needed no sync). This is the resumable checkpoint,
    /// not a migration marker.
    #[serde(default)]
    processed: HashMap<u32, String>,
    #[serde(default)]
    lua_processed: HashMap<u32, String>,
    // Legacy checkpoint for the disabled automatic workshop lane. Kept for
    // forward-compat of the JSON state file; the manual Repair Workshop
    // button does not read or write it.
    #[serde(default)]
    workshop_processed: Option<String>,
    /// AppID -> epoch seconds of the last completed Hubcap-contents pin
    /// refresh check. Throttles the pin_refresh lane (free endpoint, but the
    /// diff must not run on every poll for every game).
    #[serde(default)]
    contents_checked: HashMap<u32, u64>,
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
                "Ignoring invalid synchronizer checkpoint at {}: {}",
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

// ============================================================================
// Steam-side scans (fingerprints)
// ============================================================================

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

fn scan_app_manifests(steam_path: &str) -> HashMap<u32, String> {
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

/// Legacy scanner for the disabled automatic workshop lane. Kept so the
/// manual Repair Workshop path and future DLL-parity checks can reuse it.
#[allow(dead_code)]
fn scan_workshop_manifests(steam_path: &str) -> Option<String> {
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

// ============================================================================
// Lane helpers
// ============================================================================

#[derive(Debug, Clone)]
struct PendingTask {
    attempts: u32,
    next_attempt: Instant,
}

pub(crate) fn retry_delay(attempts: u32) -> Duration {
    let multiplier = 1u64 << attempts.min(6);
    INITIAL_RETRY_DELAY
        .checked_mul(multiplier as u32)
        .unwrap_or(MAX_RETRY_DELAY)
        .min(MAX_RETRY_DELAY)
}

/// AppIDs whose current fingerprint differs from the checkpoint's.
fn changed_against(
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

/// Takes the first task whose backoff expired, if any (one task per lane per
/// poll keeps the shared provider lane and the disk quiet).
fn take_ready(pending: &mut BTreeMap<u32, PendingTask>) -> Option<(u32, PendingTask)> {
    let now = Instant::now();
    let ready = pending
        .iter()
        .find(|(_, task)| task.next_attempt <= now)
        .map(|(app_id, _)| *app_id)?;
    pending.remove(&ready).map(|task| (ready, task))
}

fn reschedule(pending: &mut BTreeMap<u32, PendingTask>, app_id: u32, task: PendingTask) {
    let attempts = task.attempts.saturating_add(1);
    let delay = retry_delay(attempts);
    crate::desk_log_warn!(
        "hubcap-updates",
        "Task deferred app_id={} attempts={} retry_in_secs={}",
        app_id,
        attempts,
        delay.as_secs()
    );
    pending.insert(
        app_id,
        PendingTask {
            attempts,
            next_attempt: Instant::now() + delay,
        },
    );
}

fn pending_infos(pending: &BTreeMap<u32, PendingTask>) -> Vec<PendingTaskInfo> {
    pending
        .iter()
        .map(|(app_id, task)| PendingTaskInfo {
            app_id: Some(*app_id),
            attempts: task.attempts,
            next_retry_epoch: Some(now_epoch() + task.next_attempt.saturating_duration_since(Instant::now()).as_secs()),
        })
        .collect()
}

// ============================================================================
// pin_sync action (local-only)
// ============================================================================

/// Latest manifest GID Steam installed for one depot: the most recently
/// modified non-empty `<depot>_<gid>.manifest` across the two depotcache
/// folders. Local-only, no provider traffic.
pub(crate) fn latest_installed_gid(steam_path: &str, depot_id: u32) -> Option<String> {
    let prefix = format!("{depot_id}_");
    let mut best: Option<(SystemTime, String)> = None;
    for directory in [
        PathBuf::from(steam_path).join("depotcache"),
        PathBuf::from(steam_path).join("config").join("depotcache"),
    ] {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.starts_with(&prefix) || !name.ends_with(".manifest") {
                continue;
            }
            let Some(gid) = name
                .strip_prefix(&prefix)
                .and_then(|stem| stem.strip_suffix(".manifest"))
                .and_then(|gid| gid.parse::<u64>().ok())
                .filter(|gid| *gid > 0)
            else {
                continue;
            };
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            if metadata.len() == 0 {
                continue;
            }
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            if best.as_ref().map(|(time, _)| modified > *time).unwrap_or(true) {
                best = Some((modified, gid.to_string()));
            }
        }
    }
    best.map(|(_, gid)| gid)
}

/// Local-only realignment of the informational (commented) pins of one game
/// to the manifests Steam actually installed (newest `<depot>_<gid>.manifest`
/// per depot across the depotcache folders). Shared by the pin_sync lane and
/// by the "disable updates" safety net in the library command, so reactivating
/// pins can never resurrect a stale GID and downgrade the game. Returns how
/// many pins were rewritten.
///
/// Backup contract: both the pre-change and the post-change Lua are archived
/// in AetherData (`history/`, content-deduplicated) and every manifest the
/// new pins reference is copied from depotcache into `backup/<app_id>/lua/`,
/// so every caller (pin_sync lane, "disable updates" safety net) gets the
/// full contract without repeating it.
pub(crate) fn realign_pins_to_installed(steam_path: &str, app_id: u32) -> Result<usize, String> {
    let editor = LuaManifestPins::new(steam_path.to_string(), app_id);
    let content = editor.read_lua()?;

    // Candidate depots: managed by the Lua (addappid active) but with the pin
    // commented — exactly the rows `realign_commented_pins` may rewrite.
    let active_depots: Vec<u32> = LuaManifestPins::rows_for_manifest_sync(&content)
        .into_iter()
        .map(|row| row.app_id)
        .collect();
    let mut realignment = Vec::new();
    for row in LuaManifestPins::rows_from_content(&content) {
        if row.enabled || !active_depots.contains(&row.app_id) {
            continue;
        }
        if let Some(installed_gid) = latest_installed_gid(steam_path, row.app_id) {
            if installed_gid != row.manifest_id {
                realignment.push(DepotManifestPin {
                    depot_id: row.app_id,
                    manifest_id: installed_gid,
                });
            }
        }
    }
    if realignment.is_empty() {
        return Ok(0);
    }

    // Preserve the pre-change Lua exactly like the version pipelines do.
    if let Ok(backup) = crate::core::backup::GameBackup::for_app(app_id) {
        let _ = backup.store_history_version(app_id, content.as_bytes());
    }

    let realigned = editor.realign_commented_pins(&realignment)?;

    // Archive the NEW Lua too (every Lua change must land in AetherData) and
    // copy the manifests the rewritten pins reference into the backup, so a
    // future uninstall can still restore the version the user actually has.
    if let (Ok(final_lua), Ok(backup)) =
        (editor.read_lua(), crate::core::backup::GameBackup::for_app(app_id))
    {
        let _ = backup.store_history_version(app_id, final_lua.as_bytes());
        let depotcache = PathBuf::from(steam_path).join("depotcache");
        let report = backup.backup_referenced_manifests(&final_lua, &depotcache);
        if report.copied > 0 || report.errors > 0 || report.missing > 0 {
            crate::desk_log_debug!(
                "hubcap-updates",
                "Post-realign manifest backup app_id={}: {} copied, {} unchanged, {} missing, {} errors",
                app_id,
                report.copied,
                report.unchanged,
                report.missing,
                report.errors
            );
        }
    }
    Ok(realigned)
}

/// Realigns the informational (commented) pins of one updated game to the
/// manifests Steam actually installed and archives them into the AetherData
/// backup. Returns how many pins were rewritten.
async fn sync_pins_after_steam_update(app: &AppHandle, steam_path: &str, app_id: u32) -> Result<usize, String> {
    let realigned = realign_pins_to_installed(steam_path, app_id)?;
    if realigned == 0 {
        crate::desk_log_debug!(
            "hubcap-updates",
            "Pin sync found nothing to realign app_id={}",
            app_id
        );
        return Ok(0);
    }
    crate::desk_log_info!(
        "hubcap-updates",
        "Pin sync realigned app_id={} pins={}",
        app_id,
        realigned
    );

    // Lua archiving (pre AND post change) and the referenced-manifest backup
    // already happened inside realign_pins_to_installed — do not duplicate
    // the depotcache scan here.

    crate::core::library_events::notify_lua_changed(
        app,
        crate::core::library_events::LibraryChangeOrigin::Versioning,
        [app_id],
    );
    Ok(realigned)
}

// ============================================================================
// Poll loop
// ============================================================================

async fn run(app: AppHandle) {
    tokio::time::sleep(START_DELAY).await;
    crate::desk_log_info!(
        "hubcap-updates",
        "Steam-change synchronizer started (poll every {:?}, one task per lane per poll, local-first)",
        POLL_INTERVAL
    );
    with_status(|status| {
        status.running = true;
        status.started_epoch = Some(now_epoch());
    });

    let mut checkpoint = load_state();
    let mut pin_sync: BTreeMap<u32, PendingTask> = BTreeMap::new();
    let mut pin_refresh: BTreeMap<u32, PendingTask> = BTreeMap::new();
    let mut repairs: BTreeMap<u32, PendingTask> = BTreeMap::new();

    loop {
        let settings = SettingsManager::new(&app).load();
        let steam_ready = !settings.steam_path.trim().is_empty();
        let key_ready = !settings.hubcap_api_key.trim().is_empty();

        let (current_acf, current_luas) = if steam_ready {
            (
                scan_app_manifests(&settings.steam_path),
                scan_managed_luas(&settings.steam_path),
            )
        } else {
            (HashMap::new(), HashMap::new())
        };

        if !checkpoint.initialized {
            // Do not turn an existing library into work on first run: the
            // current fingerprints become the baseline. Future Steam changes,
            // or fingerprints the previous session never completed, create
            // work through the checkpoint diff below.
            // (Workshop auto-lane disabled: manual Repair Workshop only.)
            checkpoint.initialized = true;
            checkpoint.processed = current_acf.clone();
            checkpoint.lua_processed = current_luas.clone();
            if let Err(error) = save_state(&checkpoint) {
                crate::desk_log_warn!(
                    "hubcap-updates",
                    "Could not initialize the synchronizer checkpoint: {}",
                    error
                );
            }
        } else {
            let mut checkpoint_dirty = false;
            // --- pin_sync: Steam ACF changed for a game whose pins allow updates.
            let changed_acf = changed_against(&checkpoint.processed, &current_acf);
            for app_id in managed_update_candidates(changed_acf, &settings.steam_path) {
                pin_sync.entry(app_id).or_insert(PendingTask {
                    attempts: 0,
                    next_attempt: Instant::now(),
                });
            }
            // Games that need no sync must not keep re-qualifying: mark them
            // processed without doing any work.
            for (app_id, fingerprint) in &current_acf {
                if !pin_sync.contains_key(app_id)
                    && checkpoint.processed.get(app_id) != Some(fingerprint)
                    && !managed_update_candidates([*app_id], &settings.steam_path).contains(app_id)
                {
                    checkpoint.processed.insert(*app_id, fingerprint.clone());
                    checkpoint_dirty = true;
                }
            }

            // --- repair: a changed Lua is a manifest-consistency event. The
            // repair itself is local-first, but a Lua that references missing
            // manifests can only be completed with a configured key, so the
            // lane is gated on it to avoid a queue of guaranteed failures.
            if key_ready {
                for app_id in changed_against(&checkpoint.lua_processed, &current_luas) {
                    repairs.entry(app_id).or_insert(PendingTask {
                        attempts: 0,
                        next_attempt: Instant::now(),
                    });
                }
            }

            // --- pin_refresh: per-app timer diff of the Lua pins against the
            // manifests Hubcap currently packages (free contents endpoint).
            // Version-locked games exit the action before any HTTP call, so
            // queueing every managed Lua is cheap; successes (including
            // skips) advance the checkpoint timestamp.
            if key_ready {
                let now = now_epoch();
                for app_id in current_luas.keys() {
                    let last = checkpoint.contents_checked.get(app_id).copied().unwrap_or(0);
                    if now.saturating_sub(last) >= CONTENTS_REFRESH_INTERVAL.as_secs() {
                        pin_refresh.entry(*app_id).or_insert(PendingTask {
                            attempts: 0,
                            next_attempt: Instant::now(),
                        });
                    }
                }
            }

            // --- workshop: AUTOMATIC LANE DISABLED. Workshop manifests are
            // owned by AetherDLL inside Steam by default; Desk only runs
            // `sync_hubcap_workshop_manifests` when the user clicks the
            // manual Repair Workshop button in the Library.
            if checkpoint_dirty {
                let _ = save_state(&checkpoint);
            }
        }

        // ----- pin_sync execution (local-only, always allowed)
        if let Some((app_id, task)) = take_ready(&mut pin_sync) {
            match sync_pins_after_steam_update(&app, &settings.steam_path, app_id).await {
                Ok(realigned) => {
                    if let Some(fingerprint) = current_acf.get(&app_id) {
                        checkpoint.processed.insert(app_id, fingerprint.clone());
                        let _ = save_state(&checkpoint);
                    }
                    with_status(|status| {
                        status.pin_sync.processed_count += 1;
                        status.pin_sync.last_run_epoch = Some(now_epoch());
                        status.pin_sync.last_error = None;
                    });
                    crate::desk_log_info!(
                        "hubcap-updates",
                        "Pin sync complete app_id={} realigned={}",
                        app_id,
                        realigned
                    );
                }
                Err(error) => {
                    with_status(|status| {
                        status.pin_sync.last_error = Some(error.clone());
                    });
                    crate::desk_log_warn!(
                        "hubcap-updates",
                        "Pin sync failed app_id={}: {}",
                        app_id,
                        error
                    );
                    reschedule(&mut pin_sync, app_id, task);
                }
            }
        }

        // ----- pin_refresh execution (free contents diff + local-first staging)
        if let Some((app_id, task)) = take_ready(&mut pin_refresh) {
            match crate::commands::manifests::refresh_game_pins_from_hubcap(app.clone(), app_id).await {
                Ok(report) => {
                    checkpoint.contents_checked.insert(app_id, now_epoch());
                    let _ = save_state(&checkpoint);
                    with_status(|status| {
                        status.pin_refresh.processed_count += 1;
                        status.pin_refresh.last_run_epoch = Some(now_epoch());
                        status.pin_refresh.last_error = None;
                    });
                    crate::desk_log_info!(
                        "hubcap-updates",
                        "Pin refresh complete app_id={} skipped={} checked_depots={} staged={} realigned={}",
                        app_id,
                        report.skipped,
                        report.checked_depots,
                        report.staged,
                        report.realigned
                    );
                }
                Err(error) => {
                    with_status(|status| {
                        status.pin_refresh.last_error = Some(error.clone());
                    });
                    crate::desk_log_warn!(
                        "hubcap-updates",
                        "Pin refresh deferred app_id={}: {}",
                        app_id,
                        error
                    );
                    reschedule(&mut pin_refresh, app_id, task);
                }
            }
        }

        // ----- repair execution (local-first provider repair)
        if let Some((app_id, task)) = take_ready(&mut repairs) {
            match crate::commands::manifests::sync_hubcap_game_manifest(app.clone(), app_id).await {
                Ok(_) => {
                    if let Some(fingerprint) = current_luas.get(&app_id) {
                        checkpoint.lua_processed.insert(app_id, fingerprint.clone());
                        let _ = save_state(&checkpoint);
                    }
                    with_status(|status| {
                        status.repair.processed_count += 1;
                        status.repair.last_run_epoch = Some(now_epoch());
                        status.repair.last_error = None;
                    });
                    crate::desk_log_info!(
                        "hubcap-updates",
                        "Local Lua manifest repair complete app_id={}",
                        app_id
                    );
                }
                Err(error) => {
                    with_status(|status| {
                        status.repair.last_error = Some(error.clone());
                    });
                    crate::desk_log_warn!(
                        "hubcap-updates",
                        "Local Lua manifest repair deferred app_id={}: {}",
                        app_id,
                        error
                    );
                    reschedule(&mut repairs, app_id, task);
                }
            }
        }

        // ----- workshop execution: DISABLED. Manual Repair Workshop button
        // invokes `sync_hubcap_workshop_manifests` on demand; the background
        // lane never runs (AetherDLL owns Workshop manifests by default).
        with_status(|status| {
            status.last_scan_epoch = Some(now_epoch());
            status.steam_path_configured = steam_ready;
            status.hubcap_key_configured = key_ready;
            status.checkpoint_initialized = checkpoint.initialized;
            status.pin_sync.pending = pending_infos(&pin_sync);
            status.pin_refresh.pending = pending_infos(&pin_refresh);
            status.repair.pending = pending_infos(&repairs);
            // Automatic workshop lane disabled; manual Repair Workshop only.
            status.workshop.pending = Vec::new();
        });

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Starts the single process-wide synchronizer. Tauri setup calls this once.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(run(app));
}
