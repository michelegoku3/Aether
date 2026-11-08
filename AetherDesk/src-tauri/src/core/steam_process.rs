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
//!     accepted the close-Steam popup: close the running Steam games, terminate
//!     the whole Steam family and wait until the main process releases the
//!     Steam folder.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const KILL_SETTLE: Duration = Duration::from_millis(600);
const GONE_TIMEOUT: Duration = Duration::from_millis(8000);
/// Game processes are terminated directly (not asked to quit), so this only
/// covers the OS reaping them; a survivor is a real failure worth reporting.
const GAME_GONE_TIMEOUT: Duration = Duration::from_secs(3);
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

/// Directories a Steam game runs from: `<library>/steamapps/common` for the
/// main install and every extra library.
///
/// This also covers the tools Steam keeps there (Common Redistributables,
/// SteamVR): they only run while the user is using Steam, and closing them is
/// the same "Steam is going away" outcome.
fn game_roots(steam_root: &Path) -> Vec<PathBuf> {
    let roots: Vec<PathBuf> = crate::steam::library::SteamLibraryScanner::new(steam_root)
        .discover_library_paths()
        .into_iter()
        .map(|library| library.join("steamapps").join("common"))
        .filter(|path| path.is_dir())
        .collect();
    crate::desk_log_info!(
        "lifecycle",
        "Steam game roots: {}",
        roots
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    roots
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

/// True when `exe` lives under one of the game roots. Component-aware: a
/// sibling folder whose name merely starts with the same text does not match.
fn is_under_game_root(exe: &Path, roots: &[PathBuf]) -> bool {
    let exe = normalized_path(exe);
    roots.iter().any(|root| {
        let root = normalized_path(root);
        exe.len() > root.len()
            && exe.starts_with(&root)
            && exe.as_bytes().get(root.len()) == Some(&b'/')
    })
}

/// Terminate every process whose executable lives under a Steam game folder.
/// Returns how many were signalled; unkillable candidates are logged so a game
/// that stays open is visible in the lifecycle log instead of vanishing.
fn kill_steam_games(roots: &[PathBuf]) -> usize {
    if roots.is_empty() {
        return 0;
    }
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes();
    let mut killed = Vec::new();
    let mut failed = Vec::new();
    for process in sys.processes().values() {
        let Some(exe) = process.exe() else {
            continue;
        };
        if !is_under_game_root(exe, roots) {
            continue;
        }
        let name = process.name().to_string();
        if process.kill() {
            killed.push(name);
        } else {
            failed.push(name);
        }
    }
    if !failed.is_empty() {
        crate::desk_log_warn!(
            "lifecycle",
            "Could not close game process(es): {}",
            failed.join(", ")
        );
    }
    if !killed.is_empty() {
        std::thread::sleep(KILL_SETTLE);
    }
    killed.len()
}

fn snapshot_has_game(sys: &sysinfo::System, roots: &[PathBuf]) -> bool {
    sys.processes()
        .values()
        .any(|p| p.exe().is_some_and(|exe| is_under_game_root(exe, roots)))
}

/// Block until no process runs from a Steam game folder, up to
/// `GAME_GONE_TIMEOUT`. A survivor is reported: the popup promised the game
/// would close, so silently continuing would hide the failure.
fn wait_games_gone(roots: &[PathBuf]) -> bool {
    if roots.is_empty() {
        return true;
    }
    let deadline = Instant::now() + GAME_GONE_TIMEOUT;
    let mut sys = sysinfo::System::new_all();
    loop {
        sys.refresh_processes();
        if !snapshot_has_game(&sys, roots) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(GONE_POLL);
    }
}

/// Close Steam completely and wait until `steam.exe` releases the Steam folder.
/// Used before replacing files in the Steam folder, where the UI has already
/// asked the user for consent.
///
/// A game launched from a Steam library is its own process and survives a Steam
/// exit, so it is closed first (the consent popup warns the user to save). The
/// Steam helpers are terminated with the main process, but they do not gate the
/// result: a stale `steamwebhelper.exe` (or one spawned by a dying Steam) would
/// otherwise make every future operation fail after waiting out the timeout.
pub fn close_steam(steam_root: &Path) -> Result<(), String> {
    let roots = game_roots(steam_root);
    let closed = kill_steam_games(&roots);
    if closed > 0 {
        crate::desk_log_info!(
            "lifecycle",
            "Closed {} running game process(es) before Steam",
            closed
        );
    }
    if !wait_games_gone(&roots) {
        return Err("A running game did not close. Close the game and try again.".to_string());
    }

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

    #[test]
    fn a_game_process_is_recognised_by_its_library_common_folder() {
        let roots = vec![PathBuf::from(r"C:\SteamLibrary\steamapps\common")];

        assert!(is_under_game_root(
            Path::new(r"C:\SteamLibrary\steamapps\common\Skyrim\SkyrimSE.exe"),
            &roots
        ));
        // Windows paths arrive with either separator and any casing.
        assert!(is_under_game_root(
            Path::new(r"c:/steamlibrary/STEAMAPPS/Common/skyrim/SkyrimSE.exe"),
            &roots
        ));
        // A sibling folder whose name merely starts with the same text.
        assert!(!is_under_game_root(
            Path::new(r"C:\SteamLibrary\steamapps\common2\Skyrim\SkyrimSE.exe"),
            &roots
        ));
        assert!(!is_under_game_root(
            Path::new(r"C:\SteamLibrary\steamapps\common\Skyrim2\SkyrimSE.exe"),
            &[PathBuf::from(r"C:\SteamLibrary\steamapps\common\Skyrim")]
        ));
        assert!(!is_under_game_root(Path::new(r"C:\Games\game.exe"), &roots));
    }

    #[test]
    fn game_roots_cover_every_library_with_a_common_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let steam = tmp.path().join("Steam");
        let extra = tmp.path().join("Library");
        let empty = tmp.path().join("Empty");
        std::fs::create_dir_all(steam.join("steamapps").join("common")).unwrap();
        std::fs::create_dir_all(extra.join("steamapps").join("common")).unwrap();
        std::fs::create_dir_all(empty.join("steamapps")).unwrap();
        std::fs::write(
            steam.join("steamapps").join("libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"2\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                extra.display(),
                empty.display()
            ),
        )
        .unwrap();

        let roots = game_roots(&steam);

        assert!(roots.contains(&steam.join("steamapps").join("common")));
        assert!(roots.contains(&extra.join("steamapps").join("common")));
        // A library without a common folder contributes nothing.
        assert_eq!(roots.len(), 2);
    }
}
