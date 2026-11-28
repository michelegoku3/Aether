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
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use sysinfo::{Pid, System};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Copy, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SystemMemoryInfo {
    pub total_kb: u64,
    pub used_kb: u64,
    pub available_kb: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RamPreview {
    pub entries: Vec<ProcessEntry>,
    pub memory: SystemMemoryInfo,
}

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
    /// PIDs explicitly unchecked by the user in the preview modal.
    pub exclude_pids: Vec<u32>,
}

impl Default for OptimizeOptions {
    fn default() -> Self {
        Self {
            min_memory_mb: 150,
            protect_dirs: Vec::new(),
            exclude_pids: Vec::new(),
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
    "upc.exe",
    "ublauncher.exe",
    "origin.exe",
    "eaapp.exe",
    "eadesktop.exe",
    "eabackgroundservice.exe",
    "epicgameslauncher.exe",
    "battlenet.exe",
    "battle.net.exe",
    "riotclient.exe",
    "vgc.exe",
    "vgtray.exe",
    "valorant.exe",
    "gogalaxy.exe",
    "itch.exe",
    "rockstargameslauncher.exe",
    // Overlays / capture / performance / peripheral / Windows security & audio tools.
    "discord.exe",
    "overwolf.exe",
    "obs64.exe",
    "obs32.exe",
    "msafterburner.exe",
    "msiafterburner.exe",
    "rtss.exe",
    "rivatuner.exe",
    "gamebar.exe",
    "gamebarpresencewriter.exe",
    "xboxgamebarwidgets.exe",
    "nvidia share.exe",
    "nvsphelper64.exe",
    "nvcontainer.exe",
    "nvdisplay.container.exe",
    "amd radeon software.exe",
    "razer synapse.exe",
    "ghub.exe",
    "lghub.exe",
    "lghub_agent.exe",
    "steelseriesengine.exe",
    "corsair service.exe",
    "audiodg.exe",
    "msmpeng.exe",
    "securityhealthservice.exe",
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

/// Snapshot every process with its verdict and live system RAM figures.
pub fn preview(opts: &OptimizeOptions) -> RamPreview {
    let mut sys = System::new_all();
    sys.refresh_memory();
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

    RamPreview {
        entries,
        memory: SystemMemoryInfo {
            total_kb: sys.total_memory() / 1024,
            used_kb: sys.used_memory() / 1024,
            available_kb: sys.available_memory() / 1024,
        },
    }
}

/// Snapshot every process with its verdict.
pub fn plan(opts: &OptimizeOptions) -> Vec<ProcessEntry> {
    preview(opts).entries
}

struct CandidateKillOutcome {
    name: String,
    snapshot_kb: u64,
    /// `None` = exited gracefully during `WM_CLOSE`,
    /// `Some(Ok(kb))` = terminated via `kill()`,
    /// `Some(Err(()))` = refused to exit.
    post_state: Option<Result<u64, ()>>,
}

fn summarize_close_outcomes(
    outcomes: Vec<CandidateKillOutcome>,
) -> (Vec<String>, Vec<String>, u64) {
    let mut closed = Vec::new();
    let mut failed = Vec::new();
    let mut reclaimed_kb = 0u64;
    for outcome in outcomes {
        match outcome.post_state {
            None => {
                closed.push(outcome.name);
                reclaimed_kb += outcome.snapshot_kb;
            }
            Some(Ok(killed_kb)) => {
                closed.push(outcome.name);
                reclaimed_kb += killed_kb;
            }
            Some(Err(())) => failed.push(outcome.name),
        }
    }
    (closed, failed, reclaimed_kb)
}

/// Ask a process with a window to close gracefully (`WM_CLOSE`), then terminate
/// whatever survives. Returns `(closed, failed, pre_gone, reclaimed_kb)`.
fn close_candidates(candidates: &[&ProcessEntry]) -> (Vec<String>, Vec<String>, Vec<String>, u64) {
    if candidates.is_empty() {
        return (Vec::new(), Vec::new(), Vec::new(), 0);
    }

    // 0. Guard against PID reuse or processes that exited before signalling.
    let mut sys = System::new_all();
    sys.refresh_processes();
    let mut active: Vec<&ProcessEntry> = Vec::new();
    let mut pre_gone = Vec::new();
    for entry in candidates {
        let pid = Pid::from_u32(entry.pid);
        match sys.process(pid) {
            Some(proc_info) if proc_info.name().eq_ignore_ascii_case(&entry.name) => {
                active.push(*entry);
            }
            _ => pre_gone.push(entry.name.clone()),
        }
    }
    if active.is_empty() {
        return (Vec::new(), Vec::new(), pre_gone, 0);
    }

    // 1. Graceful pass through the Win32 API exposed by PowerShell. Batching
    //    keeps one hidden PowerShell start-up for the whole set.
    let ids = active
        .iter()
        .map(|entry| entry.pid.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "Get-Process -Id {ids} -ErrorAction SilentlyContinue | ForEach-Object {{ [void]$_.CloseMainWindow() }}; Start-Sleep -Milliseconds 900"
    );
    let mut ps_cmd = Command::new("powershell");
    ps_cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    #[cfg(target_os = "windows")]
    ps_cmd.creation_flags(CREATE_NO_WINDOW);
    let _ = ps_cmd.output();

    // 2. Reconcile graceful exits and terminate the survivors.
    sys.refresh_processes();
    let mut outcomes = Vec::with_capacity(active.len());
    for entry in active {
        let pid = Pid::from_u32(entry.pid);
        let post_state = match sys.process(pid) {
            Some(process) if process.name().eq_ignore_ascii_case(&entry.name) => {
                let memory_kb = process.memory() / 1024;
                if process.kill() {
                    Some(Ok(memory_kb))
                } else {
                    Some(Err(()))
                }
            }
            // Exited during WM_CLOSE (or PID already recycled): graceful close succeeded.
            _ => None,
        };
        outcomes.push(CandidateKillOutcome {
            name: entry.name.clone(),
            snapshot_kb: entry.memory_kb,
            post_state,
        });
    }
    let (closed, failed, reclaimed_kb) = summarize_close_outcomes(outcomes);
    (closed, failed, pre_gone, reclaimed_kb)
}

/// Partition `Close` entries into actionable candidates and user-excluded labels.
fn partition_close_candidates<'a>(
    entries: &'a [ProcessEntry],
    exclude_pids: &[u32],
) -> (Vec<&'a ProcessEntry>, Vec<String>) {
    let mut candidates = Vec::new();
    let mut excluded_kept = Vec::new();
    for entry in entries
        .iter()
        .filter(|entry| entry.verdict == Verdict::Close)
    {
        if exclude_pids.contains(&entry.pid) {
            excluded_kept.push(format!("{} (excluded by user)", entry.name));
        } else {
            candidates.push(entry);
        }
    }
    (candidates, excluded_kept)
}

/// Run the optimizer and return what happened. Never panics on a live system.
pub fn optimize(opts: &OptimizeOptions) -> OptimizeReport {
    let entries = plan(opts);
    let (candidates, mut kept) = partition_close_candidates(&entries, &opts.exclude_pids);

    let (closed, failed, gone, reclaimed_kb) = close_candidates(&candidates);

    // Everything that was not a close candidate is "kept"; name them for the
    // post-run summary so the user can audit the whitelist.
    for entry in entries
        .iter()
        .filter(|entry| entry.verdict == Verdict::Keep)
    {
        kept.push(format!("{} ({})", entry.name, entry.reason));
    }
    kept.sort();

    OptimizeReport {
        closed,
        failed,
        kept,
        reclaimed_kb,
        gone,
    }
}

/// Launch a Steam game through the Steam client (`steam.exe -applaunch <appid>`
/// when a valid Steam install root is configured, or the OS URL handler for
/// `steam://run/<appid>` as a fallback).
///
/// Going through Steam is deliberate: the client owns the license check and the
/// AetherDLL injection, so a directly spawned game executable would run without
/// the DRM/lua layer the library exists for.
pub fn launch_steam_game(app_id: u32, steam_path: Option<&Path>) -> Result<(), String> {
    let mut cmd = build_steam_launch_command(app_id, steam_path);
    let target = cmd.get_program().to_string_lossy().into_owned();
    crate::desk_log_info!("play", "launching app {app_id} via {target}");
    cmd.spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not launch Steam app {app_id}: {error}"))
}

/// Prefer `steam.exe -applaunch <app_id>` (silent, no cmd/Explorer window),
/// falling back to `rundll32.exe url.dll,FileProtocolHandler steam://run/<app_id>`
/// on Windows instead of `cmd /C start` (which can open Explorer in CWD).
fn build_steam_launch_command(app_id: u32, steam_path: Option<&Path>) -> Command {
    if let Some(path) = steam_path {
        let raw = path.to_string_lossy();
        if let Ok(root) = crate::steam::resolve::resolve_steam_path(&raw) {
            let mut command = Command::new(root.join("steam.exe"));
            command
                .args(["-applaunch", &app_id.to_string()])
                .current_dir(&root);
            #[cfg(target_os = "windows")]
            command.creation_flags(CREATE_NO_WINDOW);
            return command;
        }
    }

    let url = format!("steam://run/{app_id}");
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("rundll32.exe");
        command
            .args(["url.dll,FileProtocolHandler", &url])
            .creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        command.arg(&url);
        command
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut command = Command::new("xdg-open");
        command.arg(&url);
        command
    }
}

/// Protected directories for a launch: every Steam game folder (a running game
/// may hold unsaved progress) plus the running AetherDesk directory.
pub fn protected_dirs_with_app(steam_path: Option<&Path>, app_dir: Option<&Path>) -> Vec<String> {
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
    if let Some(app_dir) = app_dir {
        if app_dir.is_dir() {
            let normalized = app_dir.to_string_lossy().to_string();
            if !dirs
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&normalized))
            {
                dirs.push(normalized);
            }
        }
    }
    dirs
}

pub fn protected_dirs(steam_path: Option<&Path>) -> Vec<String> {
    let current_exe = std::env::current_exe().ok();
    protected_dirs_with_app(steam_path, current_exe.as_deref().and_then(Path::parent))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(min_memory_mb: u32, protect_dirs: &[&str]) -> OptimizeOptions {
        OptimizeOptions {
            min_memory_mb,
            protect_dirs: protect_dirs.iter().map(|dir| dir.to_string()).collect(),
            exclude_pids: Vec::new(),
        }
    }

    #[test]
    fn excluded_pids_are_kept_and_reported() {
        let entries = vec![
            ProcessEntry {
                pid: 101,
                name: "chrome.exe".to_string(),
                exe: "C:/chrome.exe".to_string(),
                memory_kb: 500 * 1024,
                user: "me".to_string(),
                verdict: Verdict::Close,
                reason: "memory-hungry".to_string(),
            },
            ProcessEntry {
                pid: 202,
                name: "code.exe".to_string(),
                exe: "C:/code.exe".to_string(),
                memory_kb: 400 * 1024,
                user: "me".to_string(),
                verdict: Verdict::Close,
                reason: "memory-hungry".to_string(),
            },
        ];
        let (candidates, kept) = partition_close_candidates(&entries, &[101]);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].pid, 202);
        assert_eq!(kept, vec!["chrome.exe (excluded by user)"]);
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
            ("MSIAfterburner.exe", "C:/MSI/MSIAfterburner.exe"),
            ("obs64.exe", "C:/OBS/obs64.exe"),
            ("EADesktop.exe", "C:/EA/EADesktop.exe"),
            ("upc.exe", "C:/Ubisoft/upc.exe"),
            ("audiodg.exe", "C:/Windows/System32/audiodg.exe"),
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

    #[test]
    fn build_steam_launch_command_prefers_steam_applaunch() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("steam.exe"), b"").unwrap();
        let command = build_steam_launch_command(730, Some(temp.path()));
        let program = command.get_program().to_string_lossy().to_lowercase();
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(program.ends_with("steam.exe"));
        assert_eq!(args, vec!["-applaunch", "730"]);
    }

    #[test]
    fn build_steam_launch_command_falls_back_without_cmd() {
        let command = build_steam_launch_command(730, None);
        let program = command.get_program().to_string_lossy().to_lowercase();
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_ne!(program, "cmd");
        #[cfg(target_os = "windows")]
        assert_eq!(args, vec!["url.dll,FileProtocolHandler", "steam://run/730"]);
    }

    #[test]
    fn protected_dirs_with_app_includes_app_dir_and_steam_common() {
        let temp = tempfile::tempdir().unwrap();
        let steam_root = temp.path().join("Steam");
        let common = steam_root.join("steamapps").join("common");
        let app_dir = temp.path().join("AetherDesk");
        std::fs::create_dir_all(&common).unwrap();
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(steam_root.join("steam.exe"), b"").unwrap();

        let dirs = protected_dirs_with_app(Some(&steam_root), Some(&app_dir));
        assert!(dirs
            .iter()
            .any(|dir| dir.eq_ignore_ascii_case(&common.to_string_lossy())));
        assert!(dirs
            .iter()
            .any(|dir| dir.eq_ignore_ascii_case(&app_dir.to_string_lossy())));
    }

    #[test]
    fn summarize_close_outcomes_counts_graceful_exits_as_reclaimed() {
        let outcomes = vec![
            CandidateKillOutcome {
                name: "chrome.exe".to_string(),
                snapshot_kb: 200 * 1024,
                post_state: None,
            },
            CandidateKillOutcome {
                name: "code.exe".to_string(),
                snapshot_kb: 300 * 1024,
                post_state: Some(Ok(310 * 1024)),
            },
            CandidateKillOutcome {
                name: "locked.exe".to_string(),
                snapshot_kb: 180 * 1024,
                post_state: Some(Err(())),
            },
        ];
        let (closed, failed, reclaimed_kb) = summarize_close_outcomes(outcomes);
        assert_eq!(closed, vec!["chrome.exe", "code.exe"]);
        assert_eq!(failed, vec!["locked.exe"]);
        assert_eq!(reclaimed_kb, 510 * 1024);
    }
}
