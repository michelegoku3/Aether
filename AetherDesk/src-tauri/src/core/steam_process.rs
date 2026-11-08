//! Steam process lifecycle helpers shared by the `start_steam` /
//! `restart_steam` commands, the AetherDLL file operations and the runtime
//! monitor.
//!
//! Single owner of "how do we spawn/kill/wait for Steam" so callers stay thin
//! and behave identically (DRY / high cohesion):
//!   * `start_steam`  -> spawn only, never kills;
//!   * `restart_steam`-> kill, WAIT for the process to actually exit, then
//!     spawn. Waiting is what prevents the double-spawn race (a second
//!     `steam.exe` launched while the first is still shutting down races the
//!     single-instance attach and is the "restart does not work well" case);
//!   * AetherDLL install/uninstall/reset -> `close_steam` after the user
//!     accepted the close-Steam popup: terminate the whole family and wait
//!     until the main process releases the Steam folder.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const KILL_SETTLE: Duration = Duration::from_millis(600);
const GONE_TIMEOUT: Duration = Duration::from_millis(8000);
const GONE_POLL: Duration = Duration::from_millis(250);

fn is_steam_process_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower == "steam.exe" || lower == "steam"
}

/// Steam web helpers. They also live in the Steam folder, so a full close
/// terminates them too. `steamservice.exe` (the Windows service) is deliberately
/// not included.
fn is_steam_helper_process_name(name: &str) -> bool {
    name.to_lowercase() == "steamwebhelper.exe"
}

fn is_steam_family_process_name(name: &str) -> bool {
    is_steam_process_name(name) || is_steam_helper_process_name(name)
}

/// True when a `System` snapshot contains a Steam main process.
pub fn snapshot_has_steam(sys: &sysinfo::System) -> bool {
    sys.processes()
        .values()
        .any(|p| is_steam_process_name(p.name()))
}

/// Fresh process snapshot for destructive migrations and DLL changes.
pub fn is_steam_running_fresh() -> bool {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();
    snapshot_has_steam(&sys)
}

/// Terminate every process matching `matches`. Returns true when at least one
/// was signalled.
fn kill_matching(matches: impl Fn(&str) -> bool) -> bool {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes();
    let mut terminated = false;
    for process in sys.processes().values() {
        if matches(process.name()) {
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

/// Kill every steam.exe process. Returns true when at least one was signalled.
pub fn kill_steam() -> bool {
    kill_matching(is_steam_process_name)
}

/// Close Steam completely and wait until `steam.exe` releases the Steam folder.
/// Used before replacing files in the Steam folder, where the UI has already
/// asked the user for consent.
///
/// The helpers are terminated with it, but they do not gate the result: a
/// stale `steamwebhelper.exe` (or one spawned by a dying Steam) would otherwise
/// make every future operation fail after waiting out the full timeout.
pub fn close_steam() -> Result<(), String> {
    kill_matching(is_steam_family_process_name);
    if !wait_steam_gone() {
        return Err("Steam did not close in time. Close Steam and try again.".to_string());
    }
    // Second pass: helpers that outlived the main process.
    kill_matching(is_steam_helper_process_name);
    Ok(())
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
    let raw = steam_dir.to_string_lossy();
    let root =
        crate::steam::resolve::resolve_steam_path(&raw).map_err(|error| error.message(&raw))?;
    let steam_exe = root.join("steam.exe");

    let mut cmd = std::process::Command::new(&steam_exe);
    cmd.current_dir(&root);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_family_matcher_covers_steam_and_its_helpers_but_not_the_service() {
        assert!(is_steam_family_process_name("steam.exe"));
        assert!(is_steam_family_process_name("Steam.exe"));
        assert!(is_steam_family_process_name("steamwebhelper.exe"));
        assert!(is_steam_family_process_name("SteamWebHelper.exe"));
        assert!(!is_steam_family_process_name("steamservice.exe"));
        assert!(!is_steam_family_process_name("game.exe"));
    }
}
