//! Pre-launch RAM optimizer.
//!
//! Closing memory-hungry processes before a game starts is only safe when the
//! processes that make the game work are never touched. This module therefore
//! owns one decision table, `decide`, with three layers:
//!
//! 1. **Ownership** — a process that is not ours (another user, SYSTEM, a
//!    service) is never signalled.
//! 2. **Denylist** — session-critical Windows processes (`csrss`, `winlogon`,
//!    `dwm`, …) are kept even when they belong to us and use RAM.
//! 3. **Whitelist** — the launcher/DLL/support chain: Steam (AetherDLL is
//!    injected into `steam.exe`), anti-cheat, overlays, launchers, our own
//!    WebView2 UI, and any process living under a Steam game folder (a running
//!    game may hold unsaved progress).
//!
//! Everything else above the memory threshold is a close candidate. The
//! threshold exists so a thousand 20 MB helpers are not killed for nothing;
//! the UI shows the plan before anything is signalled.

use serde::Serialize;
use std::path::Path;
use std::process::Command;
use sysinfo::{Pid, System};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Close,
    Keep,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessEntry {
    pub pid: u32,
    pub name: String,
    pub exe: String,
    /// Working set in kilobytes.
    pub memory_kb: u64,
    pub user: String,
    pub verdict: Verdict,
    /// Why the verdict was reached; shown in the confirmation dialog.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OptimizeReport {
    /// Names of the processes that were actually terminated.
    pub closed: Vec<String>,
    /// Close candidates that survived (no permission, self-protecting, …).
    pub failed: Vec<String>,
    /// Kept processes, with the reason (kept for the post-run summary).
    pub kept: Vec<String>,
    /// Sum of the working sets of `closed`, in kilobytes.
    pub reclaimed_kb: u64,
    /// Close candidates whose signal was not delivered because the process
    /// already exited between the snapshot and the close.
    pub gone: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct OptimizeOptions {
    /// Processes below this working set are kept (megabytes).
    pub min_memory_mb: u32,
    /// Executables under any of these directories are always kept (game
    /// installs, the AetherDesk folder).
    pub protect_dirs: Vec<String>,
}

impl Default for OptimizeOptions {
    fn default() -> Self {
        Self {
            min_memory_mb: 150,
            protect_dirs: Vec::new(),
        }
    }
}

/// Session-critical Windows processes. Never signalled, whatever the account.
const SYSTEM_DENYLIST: &[&str] = &[
    "csrss.exe",
    "winlogon.exe",
    "wininit.exe",
    "smss.exe",
    "lsass.exe",
    "services.exe",
    "svchost.exe",
    "dwm.exe",
    "explorer.exe",
    "taskhostw.exe",
    "sihost.exe",
    "fontdrvhost.exe",
    "ctfmon.exe",
    "runtimebroker.exe",
    "shellexperiencehost.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "textinputhost.exe",
    "registry",
    "system",
    "idle",
    "secure system",
    "memory compression",
];

/// Exact names that are part of the Steam / Aether / anti-cheat chain.
const KEEP_EXACT: &[&str] = &[
    // AetherDesk itself.
    "aetherdesk.exe",
    // Steam: AetherDLL is injected into the client, so the client is sacred.
    "steam.exe",
    "steamwebhelper.exe",
    "steamservice.exe",
    "steamstreamingclient.exe",
    "steamoverlayui.exe",
    // Our (and other apps') WebView2 UI: killing it kills AetherDesk's window.
    "msedgewebview2.exe",
    // Anti-cheat / integrity helpers: killing them can ban or block the game.
    "easyanticheat.exe",
    "eac.exe",
    "eac_launcher.exe",
    "beservice.exe",
    "beclient.exe",
    "battleye.exe",
    "faceitanticheat.exe",
    "faceitclient.exe",
    "vgame.exe",
    "premiumcard.exe",
    // Third-party launchers a Steam game may still need.
    "uplay.exe",
    "ublauncher.exe",
    "origin.exe",
    "eaapp.exe",
    "epicgameslauncher.exe",
    "battlenet.exe",
    "battle.net.exe",
    "riotclient.exe",
    "vgc.exe",
    "valorant.exe",
    "gogalaxy.exe",
    "itch.exe",
    "rockstargameslauncher.exe",
    // Overlays / capture / performance tools.
    "discord.exe",
    "overwolf.exe",
    "msafterburner.exe",
    "rtss.exe",
    "rivatuner.exe",
    "gamebar.exe",
    "gamebarpresencewriter.exe",
    "xboxgamebarwidgets.exe",
    "nvidia share.exe",
    "nvsphelper64.exe",
    "nvcontainer.exe",
    "amd radeon software.exe",
    "razer synapse.exe",
    "ghub.exe",
    "steelseriesengine.exe",
    "corsair service.exe",
];

/// Substrings (lower-cased) that mark a support process even when the exact
/// name differs by version suffix.
const KEEP_CONTAINS: &[&str] = &[
    "easyanticheat",
    "battleye",
    "beclient",
    "faceit",
    "anti-cheat",
    "anticheat",
    "overlayui",
    "gamebar",
    "xbox",
    "nvidia",
    "radeon",
    "amd software",
    "razer",
    "corsair",
    "logitech",
    "steelseries",
    "msedgewebview2",
    "steam",
];

fn lower(value: &str) -> String {
    value.to_lowercase()
}

fn is_system_critical(name: &str) -> bool {
    SYSTEM_DENYLIST
        .iter()
        .any(|entry| lower(entry) == lower(name))
}

fn keep_reason(name: &str, exe: &str) -> Option<&'static str> {
    let name_lower = lower(name);
    let exe_lower = lower(exe);

    if KEEP_EXACT.iter().any(|entry| lower(entry) == name_lower) {
        return Some("launcher / anti-cheat / overlay support");
    }
    if name_lower.contains("aetherdesk") || exe_lower.contains("aetherdesk") {
        return Some("AetherDesk itself");
    }
    if name_lower.contains("steam") || exe_lower.contains("steam") {
        return Some("Steam (AetherDLL runs inside it)");
    }
    if KEEP_CONTAINS.iter().any(|needle| {
        let needle = lower(needle);
        name_lower.contains(&needle) || exe_lower.contains(&needle)
    }) {
        return Some("game support / overlay");
    }
    None
}

/// True when `exe` is inside one of the protected directories.
fn under_protected_dir(exe: &str, protect_dirs: &[String]) -> bool {
    let exe_lower = lower(exe);
    protect_dirs.iter().any(|dir| {
        let dir = lower(dir.trim_end_matches(['/', '\\']));
        !dir.is_empty()
            && exe_lower.len() > dir.len()
            && exe_lower.starts_with(&dir)
            && exe_lower
                .as_bytes()
                .get(dir.len())
                .is_some_and(|sep| *sep == b'/' || *sep == b'\\')
    })
}

/// The single decision table. Pure so the matrix is unit-testable.
pub fn decide(
    name: &str,
    exe: &str,
    memory_kb: u64,
    same_user: bool,
    opts: &OptimizeOptions,
) -> (Verdict, String) {
    if !same_user {
        return (Verdict::Keep, "another user / system account".to_string());
    }
    if is_system_critical(name) {
        return (
            Verdict::Keep,
            "session-critical Windows process".to_string(),
        );
    }
    if under_protected_dir(exe, &opts.protect_dirs) {
        return (Verdict::Keep, "inside a protected game folder".to_string());
    }
    if let Some(reason) = keep_reason(name, exe) {
        return (Verdict::Keep, reason.to_string());
    }
    let threshold_kb = opts.min_memory_mb as u64 * 1024;
    if memory_kb < threshold_kb {
        return (
            Verdict::Keep,
            format!("below the {} MB threshold", opts.min_memory_mb),
        );
    }
    (
        Verdict::Close,
        "memory-hungry, not on the whitelist".to_string(),
    )
}

fn current_user(sys: &System) -> Option<String> {
    let pid = Pid::from_u32(std::process::id());
    sys.process(pid)
        .and_then(|process| process.user_id())
        .map(|uid| uid.to_string())
}

/// Snapshot every process with its verdict.
pub fn plan(opts: &OptimizeOptions) -> Vec<ProcessEntry> {
    let mut sys = System::new_all();
    sys.refresh_processes();
    let me = current_user(&sys);

    let mut entries = Vec::new();
    for (pid, process) in sys.processes() {
        let name = process.name().to_string();
        let exe = process
            .exe()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default();
        let memory_kb = process.memory() / 1024;
        let user = process
            .user_id()
            .map(|uid| uid.to_string())
            .unwrap_or_default();
        let same_user = match &me {
            Some(me) => user == *me,
            None => false,
        };
        let (verdict, reason) = decide(&name, &exe, memory_kb, same_user, opts);
        entries.push(ProcessEntry {
            pid: pid.as_u32(),
            name,
            exe,
            memory_kb,
            user,
            verdict,
            reason,
        });
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.memory_kb));
    entries
}

/// Ask a process with a window to close gracefully (`WM_CLOSE`), then terminate
/// whatever survives. Returns the names that were actually terminated.
fn close_candidates(candidates: &[&ProcessEntry]) -> (Vec<String>, Vec<String>, u64) {
    if candidates.is_empty() {
        return (Vec::new(), Vec::new(), 0);
    }

    // 1. Graceful pass through the Win32 API exposed by PowerShell. Batching
    //    keeps one PowerShell start-up for the whole set.
    let ids = candidates
        .iter()
        .map(|entry| entry.pid.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "Get-Process -Id {ids} -ErrorAction SilentlyContinue | ForEach-Object {{ [void]$_.CloseMainWindow() }}; Start-Sleep -Milliseconds 900"
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output();

    // 2. Terminate the survivors.
    let mut sys = System::new_all();
    sys.refresh_processes();
    let mut closed = Vec::new();
    let mut failed = Vec::new();
    let mut reclaimed_kb = 0u64;
    for entry in candidates {
        let pid = Pid::from_u32(entry.pid);
        match sys.process(pid) {
            Some(process) => {
                let name = process.name().to_string();
                let memory_kb = process.memory() / 1024;
                if process.kill() {
                    closed.push(name);
                    reclaimed_kb += memory_kb;
                } else {
                    failed.push(name);
                }
            }
            None => {
                // Already gone after CloseMainWindow: that is a success, but it
                // is reported separately so the summary never claims a kill it
                // did not perform.
            }
        }
    }
    (closed, failed, reclaimed_kb)
}

/// Run the optimizer and return what happened. Never panics on a live system.
pub fn optimize(opts: &OptimizeOptions) -> OptimizeReport {
    let entries = plan(opts);
    let candidates: Vec<&ProcessEntry> = entries
        .iter()
        .filter(|entry| entry.verdict == Verdict::Close)
        .collect();

    let (closed, failed, reclaimed_kb) = close_candidates(&candidates);

    // Everything that was not a close candidate is "kept"; name them for the
    // post-run summary so the user can audit the whitelist.
    let mut kept = Vec::new();
    for entry in entries
        .iter()
        .filter(|entry| entry.verdict == Verdict::Keep)
    {
        kept.push(format!("{} ({})", entry.name, entry.reason));
    }
    kept.sort();

    let gone: Vec<String> = candidates
        .iter()
        .filter(|entry| !closed.contains(&entry.name) && !failed.contains(&entry.name))
        .map(|entry| entry.name.clone())
        .collect();

    OptimizeReport {
        closed,
        failed,
        kept,
        reclaimed_kb,
        gone,
    }
}

/// Launch a Steam game through the Steam client (`steam://run/<appid>`).
///
/// Going through Steam is deliberate: the client owns the license check and the
/// AetherDLL injection, so a directly spawned executable would run without the
/// DRM/lua layer the library exists for.
pub fn launch_steam_game(app_id: u32) -> Result<(), String> {
    let url = format!("steam://run/{app_id}");
    crate::desk_log_info!("play", "launching {url}");
    Command::new("cmd")
        .args(["/C", "start", "", "", &url])
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not hand {url} to Steam: {error}"))
}

/// Protected directories for a launch: every Steam game folder (a running game
/// may hold unsaved progress) plus the AetherDesk folder.
pub fn protected_dirs(steam_path: Option<&Path>) -> Vec<String> {
    let mut dirs = Vec::new();
    if let Some(steam_path) = steam_path {
        for library in
            crate::steam::library::SteamLibraryScanner::new(steam_path).discover_library_paths()
        {
            let common = library.join("steamapps").join("common");
            if common.is_dir() {
                dirs.push(common.to_string_lossy().to_string());
            }
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(min_memory_mb: u32, protect_dirs: &[&str]) -> OptimizeOptions {
        OptimizeOptions {
            min_memory_mb,
            protect_dirs: protect_dirs.iter().map(|dir| dir.to_string()).collect(),
        }
    }

    #[test]
    fn other_accounts_are_never_touched() {
        let (verdict, reason) = decide(
            "chrome.exe",
            "C:/x/chrome.exe",
            900 * 1024,
            false,
            &opts(150, &[]),
        );
        assert_eq!(verdict, Verdict::Keep);
        assert!(reason.contains("another user"));
    }

    #[test]
    fn session_critical_processes_are_kept() {
        for name in ["csrss.exe", "winlogon.exe", "dwm.exe", "explorer.exe"] {
            let (verdict, _) = decide(
                name,
                "C:/Windows/System32/x.exe",
                2_000 * 1024,
                true,
                &opts(150, &[]),
            );
            assert_eq!(verdict, Verdict::Keep, "{name} must never be closed");
        }
    }

    #[test]
    fn launcher_and_support_chain_is_whitelisted() {
        let cases = [
            ("steam.exe", "C:/Steam/steam.exe"),
            ("steamwebhelper.exe", "C:/Steam/steamwebhelper.exe"),
            ("AetherDesk.exe", "C:/AetherDesk/AetherDesk.exe"),
            (
                "msedgewebview2.exe",
                "C:/Program Files (x86)/Microsoft/EdgeWebView/Application/158/msedgewebview2.exe",
            ),
            ("EasyAntiCheat.exe", "C:/Game/EasyAntiCheat.exe"),
            (
                "Discord.exe",
                "C:/Users/leifb/AppData/Local/Discord/Discord.exe",
            ),
        ];
        for (name, exe) in cases {
            let (verdict, _) = decide(name, exe, 900 * 1024, true, &opts(150, &[]));
            assert_eq!(verdict, Verdict::Keep, "{name} must stay");
        }
    }

    #[test]
    fn protected_game_folder_wins() {
        let protect = ["C:/Steam/steamapps/common"];
        let (verdict, reason) = decide(
            "game.exe",
            "C:/Steam/steamapps/common/SomeGame/game.exe",
            900 * 1024,
            true,
            &opts(150, &protect),
        );
        assert_eq!(verdict, Verdict::Keep);
        assert!(reason.contains("protected"));
    }

    #[test]
    fn heavy_unknown_process_above_threshold_is_closed() {
        let (verdict, reason) = decide(
            "chrome.exe",
            "C:/Program Files/Google/Chrome/Application/chrome.exe",
            800 * 1024,
            true,
            &opts(150, &[]),
        );
        assert_eq!(verdict, Verdict::Close);
        assert!(reason.contains("whitelist"));
    }

    #[test]
    fn small_processes_are_below_the_threshold() {
        let (verdict, reason) = decide(
            "helper.exe",
            "C:/x/helper.exe",
            64 * 1024,
            true,
            &opts(150, &[]),
        );
        assert_eq!(verdict, Verdict::Keep);
        assert!(reason.contains("threshold"));
    }

    #[test]
    fn protected_dir_match_is_component_aware() {
        let opts = opts(150, &["C:/games"]);
        assert!(under_protected_dir("C:/games/a/b.exe", &opts.protect_dirs));
        assert!(!under_protected_dir(
            "C:/games2/a/b.exe",
            &opts.protect_dirs
        ));
        assert!(!under_protected_dir("C:/games", &opts.protect_dirs));
    }
}
