//! Read-only adapters over the background Steam-change synchronizer.
//!
//! The monitor itself lives in `core::hubcap_update_monitor` (no Tauri
//! command logic there); these commands only expose its live state to the UI.

use crate::core::hubcap_update_monitor;

/// Live state of the background synchronizer: pending tasks per lane (with
/// attempts and next retry), completion counters, and the last failure of
/// each lane — so the exponential backoff is never invisible to the user.
#[tauri::command]
pub fn get_hubcap_monitor_status() -> hubcap_update_monitor::MonitorStatusSnapshot {
    hubcap_update_monitor::snapshot()
}
