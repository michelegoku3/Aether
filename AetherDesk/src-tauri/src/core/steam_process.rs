//! Steam process lifecycle helpers shared by the `start_steam` /
//! `restart_steam` commands and the runtime monitor.
//!
//! Single owner of "how do we spawn/kill/wait for steam.exe" so the two
//! commands stay thin and behave identically (DRY / high cohesion):
//!   * `start_steam`  -> spawn only, never kills;
//!   * `restart_steam`-> kill, WAIT for the process to actually exit, then
//!     spawn. Waiting is what prevents the double-spawn race (a second
//!     `steam.exe` launched while the first is still shutting down races the
//!     single-instance attach and is the "restart non funziona bene" case).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const KILL_SETTLE: Duration = Duration::from_millis(600);
const GONE_TIMEOUT: Duration = Duration::from_millis(8000);
const GONE_POLL: Duration = Duration::from_millis(250);

fn is_steam_process_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower == "steam.exe" || lower == "steam"
}

/// True when a `System` snapshot contains a Steam main process.
pub fn snapshot_has_steam(sys: &sysinfo::System) -> bool {
    sys.processes().values().any(|p| is_steam_process_name(p.name()))
}

/// Kill every steam.exe process. Returns true when at least one was signalled.
pub fn kill_steam() -> bool {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes();
    let mut terminated = false;
    for process in sys.processes().values() {
        if is_steam_process_name(process.name()) {
            let _ = process.kill();
            terminated = true;
        }
    }
    if terminated {
        // Give the OS a moment to actually reap the process before we check.
        std::thread::sleep(KILL_SETTLE);
    }
    terminated
}

/// Block until no steam.exe process is visible (polling), up to `GONE_TIMEOUT`.
pub fn wait_steam_gone() -> bool {
    let deadline = Instant::now() + GONE_TIMEOUT;
    let mut sys = sysinfo::System::new_all();
    loop {
        sys.refresh_processes();
        if !snapshot_has_steam(&sys) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(GONE_POLL);
    }
}

/// Spawn `steam.exe` from the given Steam root in a new console (detached).
pub fn spawn_steam(steam_dir: &Path) -> Result<PathBuf, String> {
    if !steam_dir.exists() {
        return Err("Steam installation path does not exist. Please check your settings.".to_string());
    }
    let steam_exe = steam_dir.join("steam.exe");
    if !steam_exe.exists() {
        return Err(format!("steam.exe was not found in Steam directory: {:?}", steam_exe));
    }

    let mut cmd = std::process::Command::new(&steam_exe);
    cmd.current_dir(steam_dir);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NEW_CONSOLE: the game/Steam UI is independent of AetherDesk.
        cmd.creation_flags(0x08000000);
    }
    cmd.spawn()
        .map_err(|e| format!("Failed to launch Steam process: {e}"))?;
    Ok(steam_exe)
}
