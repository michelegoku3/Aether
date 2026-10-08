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

/// Canale push dello stato del sincronizzatore.
///
/// Emesso solo quando cambia la parte **significativa** dello snapshot:
/// composizione delle code, tentativi, task abbandonati, contatori di
/// completamento, ultimo errore, flag di readiness. Gli orologi puri
/// (`last_scan_epoch`, `started_epoch`, `last_run_epoch`, `next_retry_epoch`)
/// sono esclusi dalla firma di proposito: cambiano ad ogni poll (20 s) e
/// trasformerebbero il push in un secondo polling. Il popup li aggiorna con il
/// proprio poll lento di recovery — push per gli eventi, poll per i countdown.
pub const SYNC_STATUS_EVENT: &str = "sync://status-changed";

fn status_store() -> &'static Mutex<MonitorStatusSnapshot> {
    static STORE: OnceLock<Mutex<MonitorStatusSnapshot>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(MonitorStatusSnapshot::default()))
}

/// Handle per il canale push, registrato da [`start`]. `with_status` viene
/// chiamato anche da percorsi che non hanno un `AppHandle` in scope: quando la
/// cella è vuota l'emit viene semplicemente saltato (il polling del client
/// resta la rete di sicurezza), quindi l'assenza non è mai un errore.
pub(super) fn status_handle() -> &'static OnceLock<AppHandle> {
    static HANDLE: OnceLock<AppHandle> = OnceLock::new();
    &HANDLE
}

/// Ultima firma già pubblicata, per non ri-emettere uno snapshot identico.
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

/// Impronta confrontabile di tutto ciò che il popup mostra, orologi esclusi.
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

/// Pubblica lo snapshot sul canale push. Chiamata SENZA il lock dello stato:
/// `emit` serializza e consegna alle webview, e non deve mai avvenire dentro
/// una sezione critica che altri thread usano per aggiornare le code.
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
    // Payload da pubblicare, calcolato dentro il lock ma emesso fuori.
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

