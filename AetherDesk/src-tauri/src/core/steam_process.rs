//! Steam process lifecycle helpers shared by the `start_steam` /
//! `restart_steam` commands, the AetherDLL file operations and the runtime
//! monitor.
//!
//! Single owner of "how do we spawn/kill/wait for Steam" so callers stay thin
//! and behave identically (DRY / high cohesion):
//!   * `start_steam`  -> spawn only, never kills;
//!   * `restart_steam`-> kill the whole family, WAIT until a fresh snapshot has
//!     held no member for two consecutive polls, then spawn. Waiting is what
//!     prevents the double-spawn race: a second `steam.exe` launched while the
//!     first is still shutting down races the single-instance attach, and a
//!     client that starts while the previous `steamwebhelper.exe` (the CEF
//!     children share its image name) still holds the client pipe and the CEF
//!     cache handshake aborts seconds later on `stalled cross-thread pipe` —
//!     the "restart does not work well" case in both of its forms. One clean
//!     poll is not enough: the dying client can spawn a helper right after it,
//!     and a family that survives the kill budget is reported, never spawned
//!     onto;
//!   * AetherDLL install/uninstall/reset -> `close_steam` after the user
//!     accepted the close-Steam popup: close the running Steam games, terminate
//!     the whole Steam family and wait until the main process releases the
//!     Steam folder.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const KILL_SETTLE: Duration = Duration::from_millis(600);
const GONE_TIMEOUT: Duration = Duration::from_millis(8000);
/// Game processes are terminated directly (not asked to quit), so this only
/// covers the OS reaping them; a survivor is a real failure worth reporting.
const GAME_GONE_TIMEOUT: Duration = Duration::from_secs(3);
const GONE_POLL: Duration = Duration::from_millis(250);
/// Consecutive clean polls required before the Steam family counts as gone.
///
/// A single clean poll is not enough: the client can spawn a helper between the
/// kill and the poll, and a client started against that helper aborts on
/// `stalled cross-thread pipe` seconds later. Two polls a `GONE_POLL` apart
/// prove the family stayed empty across a real time window.
const GONE_CLEAN_POLLS: u32 = 2;
/// Budget for the kill loop: long enough to catch the helper a dying client
/// spawns after the first snapshot.
const KILL_TIMEOUT: Duration = Duration::from_millis(4000);

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

/// True when a `System` snapshot still holds any member of the Steam family: the
/// client plus its web helpers. The CEF children share the helper's image name,
/// so one name covers them all.
pub fn snapshot_has_steam_family(sys: &sysinfo::System) -> bool {
    names_have_steam_family(sys.processes().values().map(|p| p.name().to_string()))
}

/// Pure predicate behind [`snapshot_has_steam_family`]: the wait must keep
/// polling while *any* family member is alive, so this is what the tests pin.
fn names_have_steam_family(names: impl IntoIterator<Item = String>) -> bool {
    names
        .into_iter()
        .any(|name| is_steam_family_process_name(&name))
}

/// Pure decision behind [`wait_steam_family_gone`]: counts consecutive polls
/// that saw no family member and reports when the run is long enough to spawn.
#[derive(Debug, Default)]
struct GoneWatch {
    clean: u32,
}

impl GoneWatch {
    /// Records one snapshot. Returns true when the family is gone for good.
    fn observe(&mut self, family_present: bool) -> bool {
        if family_present {
            self.clean = 0;
            false
        } else {
            self.clean += 1;
            self.clean >= GONE_CLEAN_POLLS
        }
    }
}

/// Names and pids of the family members in a snapshot, for the timeout log.
fn family_names(sys: &sysinfo::System) -> Vec<String> {
    let mut names: Vec<String> = sys
        .processes()
        .values()
        .filter(|process| is_steam_family_process_name(process.name()))
        .map(|process| format!("{}({})", process.name(), process.pid().as_u32()))
        .collect();
    names.sort();
    names
}

/// Fresh process snapshot for destructive migrations and DLL changes.
pub fn is_steam_running_fresh() -> bool {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();
    snapshot_has_steam(&sys)
}

/// What the kill loop does after one pass.
#[derive(Debug, PartialEq, Eq)]
enum KillStep {
    /// The snapshot held no match: the kill is complete.
    Done,
    /// Someone is still alive and the budget allows another pass.
    Again,
    /// Someone survived the whole budget: report them instead of spinning.
    GiveUp,
}

/// Pure termination rule for [`kill_matching`], so a survivor can never turn
/// the loop into a hang.
fn kill_step(alive: usize, elapsed: Duration, budget: Duration) -> KillStep {
    if alive == 0 {
        KillStep::Done
    } else if elapsed >= budget {
        KillStep::GiveUp
    } else {
        KillStep::Again
    }
}

/// Whether the caller may spawn a new client after a kill attempt.
///
/// A kill that signalled someone but left a family member behind is the exact
/// state that makes the next client abort on `stalled cross-thread pipe`: the
/// caller must refuse and report, never "proceed anyway".
pub fn spawn_allowed(killed: bool, family_gone: bool) -> bool {
    !killed || family_gone
}

/// Terminate every process matching `matches`, re-snapshotting until none is
/// left. Returns true when at least one was signalled.
///
/// One pass is not enough: a dying client can spawn a helper after the first
/// snapshot, and that helper is exactly what makes the next client abort on
/// `stalled cross-thread pipe`. The pass repeats until a fresh snapshot is
/// empty, mirroring `close_running_games`.
fn kill_matching(matches: impl Fn(&str) -> bool) -> bool {
    let started = Instant::now();
    let mut sys = sysinfo::System::new_all();
    let mut signalled: BTreeSet<String> = BTreeSet::new();
    let mut passes = 0u32;
    loop {
        sys.refresh_processes();
        passes += 1;
        let mut alive: BTreeSet<String> = BTreeSet::new();
        for process in sys.processes().values() {
            if matches(process.name()) {
                let label = format!("{}({})", process.name(), process.pid().as_u32());
                alive.insert(label.clone());
                let _ = process.kill();
                signalled.insert(label);
            }
        }
        match kill_step(alive.len(), started.elapsed(), KILL_TIMEOUT) {
            KillStep::Done => break,
            KillStep::GiveUp => {
                crate::desk_log_warn!(
                    "lifecycle",
                    "Steam family kill: still present after {} ms ({} pass(es)): {}",
                    started.elapsed().as_millis(),
                    passes,
                    alive.into_iter().collect::<Vec<_>>().join(", ")
                );
                break;
            }
            KillStep::Again => std::thread::sleep(GONE_POLL),
        }
    }
    if !signalled.is_empty() {
        crate::desk_log_info!(
            "lifecycle",
            "Steam family kill: signalled {} process(es) in {} pass(es): {}",
            signalled.len(),
            passes,
            signalled.iter().cloned().collect::<Vec<_>>().join(", ")
        );
        // Give the OS a moment to release the pipe and cache handles, not just
        // to drop the processes from the snapshot.
        std::thread::sleep(KILL_SETTLE);
    }
    !signalled.is_empty()
}

/// Kill every process of the Steam family (client + web helpers). Returns true
/// when at least one was signalled.
///
/// The helpers are part of the kill, not an afterthought: one left alive keeps
/// the client pipe and the CEF cache handshake, so the next client would start
/// against a half-dead peer.
pub fn kill_steam_family() -> bool {
    kill_matching(is_steam_family_process_name)
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

/// Close every process whose executable lives under a Steam game folder and
/// wait until they are gone. Returns the names that were matched.
///
/// The kill is retried on every poll: a bootstrapper can spawn the real game
/// executable after the first snapshot, and a self-restarting Steam helper must
/// not block the operation. A process that survives the whole window is named
/// in the error: the popup promised the game would close, so silently
/// continuing would hide the failure.
fn close_running_games(roots: &[PathBuf]) -> Result<Vec<String>, String> {
    if roots.is_empty() {
        return Ok(Vec::new());
    }
    let deadline = Instant::now() + GAME_GONE_TIMEOUT;
    let mut sys = sysinfo::System::new_all();
    let mut matched = BTreeSet::new();
    loop {
        sys.refresh_processes();
        let mut alive = BTreeSet::new();
        for process in sys.processes().values() {
            let Some(exe) = process.exe() else {
                continue;
            };
            if !is_under_game_root(exe, roots) {
                continue;
            }
            let name = process.name().to_string();
            matched.insert(name.clone());
            alive.insert(name);
            let _ = process.kill();
        }
        if alive.is_empty() {
            return Ok(matched.into_iter().collect());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "These processes did not close: {}. Close them and try again.",
                alive.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        std::thread::sleep(GONE_POLL);
    }
}

/// Close Steam completely and wait until the Steam folder is free again.
/// Used before replacing files in the Steam folder, where the UI has already
/// asked the user for consent.
///
/// A game launched from a Steam library is its own process and survives a Steam
/// exit, so it is closed first (the consent popup warns the user to save). The
/// web helpers are terminated and waited for like the client: they keep cache
/// and pipe state inside the same folder, so their exit is part of "closed".
pub fn close_steam(steam_root: &Path) -> Result<(), String> {
    let roots = game_roots(steam_root);
    let closed = close_running_games(&roots)?;
    if !closed.is_empty() {
        crate::desk_log_info!(
            "lifecycle",
            "Closed {} game process(es) before Steam: {}",
            closed.len(),
            closed.join(", ")
        );
    }

    kill_matching(is_steam_family_process_name);
    // The wait covers the helpers too: they hold the very files being replaced,
    // so "steam.exe is gone" alone is not a closed Steam.
    if !wait_steam_family_gone() {
        return Err("Steam did not close in time. Close Steam and try again.".to_string());
    }

    // One last sweep: a game can finish bootstrapping while Steam shuts down,
    // after the pre-kill snapshot was clean.
    let late = close_running_games(&roots)?;
    if !late.is_empty() {
        crate::desk_log_info!(
            "lifecycle",
            "Closed {} game process(es) spawned during the Steam shutdown: {}",
            late.len(),
            late.join(", ")
        );
    }
    Ok(())
}

/// Block until no member of the Steam family is visible for
/// [`GONE_CLEAN_POLLS`] consecutive polls, up to `GONE_TIMEOUT`.
pub fn wait_steam_family_gone() -> bool {
    let started = Instant::now();
    let deadline = started + GONE_TIMEOUT;
    let mut sys = sysinfo::System::new_all();
    let mut watch = GoneWatch::default();
    loop {
        sys.refresh_processes();
        let present = snapshot_has_steam_family(&sys);
        if watch.observe(present) {
            crate::desk_log_info!(
                "lifecycle",
                "wait_steam_family_gone: gone after {} ms ({} clean polls).",
                started.elapsed().as_millis(),
                GONE_CLEAN_POLLS
            );
            return true;
        }
        if Instant::now() >= deadline {
            crate::desk_log_warn!(
                "lifecycle",
                "wait_steam_family_gone: still present after {} ms: {}",
                started.elapsed().as_millis(),
                family_names(&sys).join(", ")
            );
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
    fn the_family_wait_stays_awake_while_a_web_helper_is_alive() {
        // The restart race: steam.exe is gone but a helper is still shutting
        // down, so the wait must not report the family as closed.
        assert!(names_have_steam_family(vec![
            "steamwebhelper.exe".to_string()
        ]));
        assert!(names_have_steam_family(vec![
            "explorer.exe".to_string(),
            "SteamWebHelper.exe".to_string(),
        ]));
        assert!(!names_have_steam_family(Vec::new()));
        // The Windows service outlives every client by design.
        assert!(!names_have_steam_family(vec![
            "steamservice.exe".to_string(),
            "explorer.exe".to_string(),
        ]));
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
    fn a_kill_pass_stops_on_an_empty_snapshot_or_on_the_deadline() {
        let budget = Duration::from_secs(4);
        assert_eq!(kill_step(0, Duration::ZERO, budget), KillStep::Done);
        assert_eq!(
            kill_step(1, Duration::from_secs(1), budget),
            KillStep::Again,
            "a survivor inside the budget earns another pass"
        );
        assert_eq!(
            kill_step(1, budget, budget),
            KillStep::GiveUp,
            "a survivor at the deadline is reported, not retried forever"
        );
    }

    #[test]
    fn a_new_client_is_spawned_only_when_the_previous_family_is_gone() {
        // Spawning onto a live family is the crash this module prevents: the
        // caller must refuse instead of "proceeding anyway".
        assert!(spawn_allowed(false, false), "Steam was not running");
        assert!(spawn_allowed(true, true), "the family was confirmed gone");
        assert!(
            !spawn_allowed(true, false),
            "kill signalled someone but the family is still present"
        );
    }

    #[test]
    fn one_clean_poll_does_not_count_the_steam_family_as_gone() {
        // The restart race: the client dies, the first poll sees nothing, and a
        // helper the dying client spawned shows up right after. Releasing the
        // spawn on that poll is what makes the new client abort on
        // "stalled cross-thread pipe"; the family must stay empty for two
        // polls a GONE_POLL apart.
        let mut watch = GoneWatch::default();
        assert!(
            !watch.observe(false),
            "the first clean poll must not release the spawn"
        );
        assert!(
            watch.observe(false),
            "two consecutive clean polls release the spawn"
        );
    }

    #[test]
    fn a_steam_member_seen_again_restarts_the_clean_poll_run() {
        let mut watch = GoneWatch::default();
        assert!(!watch.observe(false));
        assert!(
            !watch.observe(true),
            "a helper seen again must restart the run"
        );
        assert!(!watch.observe(false));
        assert!(watch.observe(false));
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
