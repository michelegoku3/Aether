//! Background synchronizer for Steam-side changes.
//!
//! One process-wide poller owns four independently paced lanes:
//! - `pin_sync`: local realignment to Steam's installed manifests;
//! - `pin_refresh`: provider contents diff for updates-enabled games;
//! - `repair`: local-first repair after managed Lua changes;
//! - `workshop`: local-first Workshop manifest staging.
//!
//! The first scan establishes a durable baseline. Later fingerprints enqueue
//! work; each lane runs at most one task per poll and exposes queue, retry and
//! unresolved state to the UI.
//!
//! Module ownership:
//! - `orchestrator`: scan/enqueue/execute loop;
//! - `runtime`: explicit in-memory lane state;
//! - `scheduler`: deterministic queues and bounded retry policy;
//! - `lanes`: execution and success/failure transitions;
//! - `candidates`: pin-refresh cadence and eligibility;
//! - `installed_state`: local manifest evidence and pin realignment;
//! - `scans`: Steam fingerprints and durable checkpoint;
//! - `status`: UI snapshot and push event.

mod candidates;
mod installed_state;
mod lanes;
mod orchestrator;
mod runtime;
mod scans;
mod scheduler;
mod status;

pub use status::{snapshot, LaneStatus, MonitorStatusSnapshot, PendingTaskInfo};

pub(crate) use installed_state::{latest_installed_gid, realign_pins_to_installed};

#[cfg(test)]
pub(crate) use candidates::pin_refresh_candidates;
#[cfg(test)]
pub(crate) use installed_state::{acf_installed_gid, installed_gid_for_depot};
#[cfg(test)]
pub(crate) use scheduler::{
    reschedule, retry_delay, PendingTask, INITIAL_RETRY_DELAY, MAX_RETRY_DELAY,
    MAX_TASK_ATTEMPTS,
};

use status::status_handle;
use tauri::AppHandle;

/// Starts the single process-wide synchronizer. Tauri setup calls this once.
pub fn start(app: AppHandle) {
    let _ = status_handle().set(app.clone());
    tauri::async_runtime::spawn(orchestrator::run(app));
}
