//! Synchronizer state exposed to the UI and its deduplicated push channel.
//!
//! Owns only the snapshot store, significant-state signature and Tauri emit.
//! Poll orchestration and lane execution live in sibling modules.

use serde::Serialize;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

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
    /// Tasks dropped after `MAX_TASK_ATTEMPTS` failed attempts. They are no
    /// longer retried on a timer; the next Steam-side change (or an app
    /// restart) queues them again. Surfaced so the UI can show "gave up after
    /// N tries" instead of an eternally pending task.
    #[serde(default)]
    pub unresolved: Vec<PendingTaskInfo>,
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

/// Push channel for the synchronizer state.
///
/// Emitted only when the **significant** part of the snapshot changes: queue
/// composition, attempts, abandoned tasks, completion counters, last error,
/// readiness flags. Pure clocks (`last_scan_epoch`, `started_epoch`,
/// `last_run_epoch`, `next_retry_epoch`) are excluded from the signature on
/// purpose: they change on every poll (20 s) and would turn the push into a
/// second polling loop. The popup refreshes them with its own slow recovery
/// poll — push for events, poll for countdowns.
pub const SYNC_STATUS_EVENT: &str = "sync://status-changed";

fn status_store() -> &'static Mutex<MonitorStatusSnapshot> {
    static STORE: OnceLock<Mutex<MonitorStatusSnapshot>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(MonitorStatusSnapshot::default()))
}

/// Handle for the push channel, registered by [`start`]. `with_status` is also
/// called from paths with no `AppHandle` in scope: when the cell is empty the
/// emit is simply skipped (client polling remains the safety net), so absence
/// is never an error.
pub(super) fn status_handle() -> &'static OnceLock<AppHandle> {
    static HANDLE: OnceLock<AppHandle> = OnceLock::new();
    &HANDLE
}

/// Last already-published signature, to avoid re-emitting an identical snapshot.
fn last_signature() -> &'static Mutex<Option<String>> {
    static SIGNATURE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    SIGNATURE.get_or_init(|| Mutex::new(None))
}

fn task_key(task: &PendingTaskInfo) -> String {
    format!(
        "{}:{}",
        task.app_id.map(|id| id.to_string()).unwrap_or_else(|| "-".to_string()),
        task.attempts
    )
}

fn lane_signature(label: &str, lane: &LaneStatus) -> String {
    let keys = |tasks: &[PendingTaskInfo]| -> String {
        tasks.iter().map(task_key).collect::<Vec<_>>().join(",")
    };
    format!(
        "{}[{}][{}]{}/{}",
        label,
        keys(&lane.pending),
        keys(&lane.unresolved),
        lane.processed_count,
        lane.last_error.as_deref().unwrap_or("")
    )
}

/// Comparable fingerprint of everything the popup shows, clocks excluded.
fn status_signature(status: &MonitorStatusSnapshot) -> String {
    format!(
        "run={}|steam={}|key={}|ckpt={}|{}|{}|{}|{}",
        status.running,
        status.steam_path_configured,
        status.hubcap_key_configured,
        status.checkpoint_initialized,
        lane_signature("sync", &status.pin_sync),
        lane_signature("refresh", &status.pin_refresh),
        lane_signature("repair", &status.repair),
        lane_signature("workshop", &status.workshop),
    )
}

/// Publishes the snapshot on the push channel. Called WITHOUT holding the state
/// lock: `emit` serializes and delivers to the webviews, and must never run
/// inside a critical section other threads use to update the queues.
fn publish_status(payload: MonitorStatusSnapshot) {
    let Some(app) = status_handle().get() else {
        return;
    };
    if let Err(error) = app.emit(SYNC_STATUS_EVENT, payload) {
        crate::desk_log_warn!(
            "hubcap-updates",
            "Could not emit sync status event: {}",
            error
        );
    }
}

pub(super) fn now_epoch() -> u64 {
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

pub(super) fn with_status(update: impl FnOnce(&mut MonitorStatusSnapshot)) {
    // Payload to publish, computed inside the lock but emitted outside.
    let changed = {
        let Ok(mut status) = status_store().lock() else {
            return;
        };
        update(&mut status);
        let signature = status_signature(&status);
        let Ok(mut last) = last_signature().lock() else {
            return;
        };
        if last.as_deref() == Some(signature.as_str()) {
            None
        } else {
            *last = Some(signature);
            Some(status.clone())
        }
    };
    if let Some(payload) = changed {
        publish_status(payload);
    }
}

