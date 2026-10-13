//! Steam process monitor: a single shared poller that keeps the
//! "Steam running" state in memory and notifies the UI ONLY on changes.
//!
//! Design: the process scan (sysinfo) is the ground truth for
//! "is steam.exe running or not" and is relatively expensive, so it must NOT
//! happen on every render/click ("is it running?" → O(1) read). A dedicated
//! thread runs it every tick (1 s: Start/Restart/exit transitions are perceived
//! in real time for the Sidebar label, and the cost stays at a few ms); the
//! state lives in a global `AtomicBool` read by independent commands and
//! watchers. When the state changes, the tauri `steam://runtime-state` event
//! is emitted with the new boolean (`bool` payload).
//!
//! The start/restart commands update the state optimistically
//! (`mark`) right after kill/spawn, so the label converges instantly;
//! the poller remains the safety net for external transitions
//! (manual Steam close, downstream failed spawn).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter};

/// Event name emitted to UI listeners on every state transition.
pub const STEAM_RUNTIME_EVENT: &str = "steam://runtime-state";

/// Poller interval: reactive to changes (transitions in ~1 s) but
/// negligible as load (one process scan per second).
const POLL_INTERVAL: Duration = Duration::from_millis(1000);

static STEAM_RUNNING: AtomicBool = AtomicBool::new(false);
static MONITOR_STARTED: AtomicBool = AtomicBool::new(false);

/// True when the monitor's last scan found Steam running.
/// O(1) read: runs no scan (that belongs to the poller thread).
pub fn is_steam_running() -> bool {
    STEAM_RUNNING.load(Ordering::Relaxed)
}

/// Opportunistic update from the start/restart commands: after a kill or a
/// successful spawn the UI reacts immediately, the poller corrects any errors
/// on the next scan.
pub fn mark(running: bool) {
    STEAM_RUNNING.store(running, Ordering::Relaxed);
}

/// Starts the poller (idempotent: further calls are no-ops) and runs an
/// initial synchronous scan right away so the state is correct even before
/// the first tick — no UI "loading" window after opening.
pub fn start(app: AppHandle) {
    if MONITOR_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    // Initial SYNCHRONOUS scan: the state is correct before the webview even
    // mounts, so the first `is_steam_running` (Start/Restart label at boot)
    // already reads the right value when Steam is open.
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes();
    STEAM_RUNNING.store(crate::core::steam_process::snapshot_has_steam(&sys), Ordering::Relaxed);

    let handle = app;
    std::thread::Builder::new()
        .name("steam-monitor".to_string())
        .spawn(move || {
            crate::desk_log_info!(
                "lifecycle",
                "Steam process monitor started (poll every {:?})",
                POLL_INTERVAL
            );
            let mut sys = sysinfo::System::new_all();
            let mut first = true;
            loop {
                sys.refresh_processes();
                let running = crate::core::steam_process::snapshot_has_steam(&sys);
                let prev = STEAM_RUNNING.swap(running, Ordering::Relaxed);
                if running != prev || first {
                    // First tick: always emit, so the UI converges immediately
                    // even when the initial state matches the default.
                    if !first {
                        crate::desk_log_info!(
                            "lifecycle",
                            "Steam process state changed: {} -> {}",
                            if prev { "running" } else { "stopped" },
                            if running { "running" } else { "stopped" }
                        );
                    }
                    let _ = handle.emit(STEAM_RUNTIME_EVENT, running);
                    first = false;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        })
        .expect("failed to spawn steam-monitor thread");
}
