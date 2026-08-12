// Windows Defender exclusion helpers.
//
// AetherDesk applies crack files into Steam library folders, which Windows
// Defender may flag as false positives and silently quarantine. The exclusion
// set therefore covers:
//
//   1. The app processes (`AetherDesk.exe`, `aether_updater.exe`) excluded BY
//      NAME, so they are ignored regardless of where the portable folder lives
//      — the exclusion survives moving the folder around. This is the durable
//      fix for a portable distribution.
//   2. `steam.exe` excluded BY NAME: since Steam is the process that loads the
//      cracked games, this covers every library (main + extras on any drive)
//      without needing to enumerate paths one by one.
//   3. Every Steam library folder discovered from the configured Steam path
//      (main install + any extra libraries registered in libraryfolders.vdf),
//      excluded by absolute path as a second layer.
//
// Because the app runs with admin rights, exclusions are applied via
// PowerShell (`Add-MpPreference`) with a hidden console window.
use crate::core::paths::LocalAppPaths;
use crate::core::settings::SettingsManager;
use crate::steam::library::SteamLibraryScanner;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

/// Whether the antivirus-exclusion prompt has already been handled by the user.
#[tauri::command]
pub fn get_antivirus_exclusion_done(app: tauri::AppHandle) -> Result<bool, String> {
    Ok(SettingsManager::new(&app).load().antivirus_exclusion_done)
}

/// Persist that the user has handled the exclusion (used by "I added it
/// manually" / when the automatic PowerShell succeeded).
#[tauri::command]
pub fn acknowledge_antivirus_exclusion(app: tauri::AppHandle) -> Result<(), String> {
    let manager = SettingsManager::new(&app);
    let mut settings = manager.load();
    settings.antivirus_exclusion_done = true;
    manager.save(&settings)
}

/// Collect every folder that needs a Windows Defender exclusion.
///
/// Returns a de-duplicated list of folders to exclude by absolute path:
///   - The AetherDesk install root (covers the current folder; the durable,
///     location-independent protection comes from the process exclusions).
///   - The configured Steam path.
///   - Every extra Steam library discovered from `libraryfolders.vdf`.
fn collect_exclusion_paths(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();

    let mut push = |path: PathBuf| {
        if !path.is_dir() {
            return;
        }
        let key = path
            .to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase();
        if seen.insert(key) {
            paths.push(path);
        }
    };

    // 1. AetherDesk install folder.
    push(LocalAppPaths::install_root());

    // 2. Steam folders (main path + every library from libraryfolders.vdf).
    let settings = SettingsManager::new(app).load();
    let steam_path = settings.steam_path.trim();
    if !steam_path.is_empty() {
        // `SteamLibraryScanner::new` accepts Option<String> for the active
        // library and internally filters out empty strings, so wrapping in
        // Some() is always correct here — even when active_library is "".
        let active_library = if settings.active_library.trim().is_empty() {
            None
        } else {
            Some(settings.active_library.clone())
        };
        let scanner = SteamLibraryScanner::new(steam_path, active_library);
        for library in scanner.discover_library_paths() {
            push(library);
        }
    }

    paths
}

/// The process names to exclude by name (not path). Excluding a process by its
/// *file name* makes Windows Defender ignore it **regardless of where it runs
/// from**, which is exactly what a portable app that can be moved between
/// folders needs: the exclusion stays valid no matter where the ZIP is unpacked.
///
/// - `AetherDesk.exe` / `aether_updater.exe`: the app and its temporary updater
///   copy (the latter runs from a temp path under AetherData).
/// - `steam.exe`: covers every game process Steam loads, so cracks / manifests /
///   DLLs written into **any** Steam library (including extra libraries on other
///   drives) are not flagged — the exclusion follows Steam regardless of where
///   each library lives.
const APP_PROCESS_EXCLUSIONS: [&str; 3] = [
    "AetherDesk.exe",
    "aether_updater.exe",
    "steam.exe",
];

/// Try to add the relevant exclusions to Windows Defender via PowerShell
/// (hidden window, admin rights already present). On success the prompt is
/// marked as done.
///
/// The exclusions are two kinds:
///   - *Process* exclusions (`AetherDesk.exe`, `aether_updater.exe`, `steam.exe`):
///     matched by file name, so they follow the app and Steam wherever they live
///     — no re-detection needed. This is the durable fix, and it covers every
///     Steam library (including extra ones on other drives).
///   - *Path* exclusions for the Steam library folders: a second layer where
///     crack / manifest / DLL files are written, so even files on disk (not just
///     ones being touched by a running process) are protected.
#[tauri::command]
pub fn apply_antivirus_exclusion(app: tauri::AppHandle) -> Result<String, String> {
    let folders = collect_exclusion_paths(&app);

    // Build a single PowerShell script that adds every exclusion in one
    // invocation, so the user sees only one UAC/PowerShell flash instead of N.
    let mut script = Vec::new();

    // 1. Process exclusions — location-independent, cover the app + updater.
    for process in &APP_PROCESS_EXCLUSIONS {
        script.push(format!(
            "Add-MpPreference -ExclusionProcess '{}' -ErrorAction Stop",
            process
        ));
    }

    // 2. Path exclusions — Steam library folders (and the current install root).
    for path in &folders {
        let escaped = path.to_string_lossy().replace('\'', "''");
        script.push(format!(
            "Add-MpPreference -ExclusionPath '{}' -ErrorAction Stop",
            escaped
        ));
    }

    let script = script.join("; ");
    crate::desk_log_info!("antivirus", "Applying Windows Defender exclusions for {} folders and {} processes", folders.len(), APP_PROCESS_EXCLUSIONS.len());

    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script]);
    hide_window(&mut command);

    let output = command
        .output()
        .map_err(|error| format!("Failed to launch PowerShell: {}", error))?;

    if output.status.success() {
        acknowledge_antivirus_exclusion(app)?;
        crate::desk_log_info!("antivirus", "Successfully applied Windows Defender exclusions");
        let mut summary: Vec<String> = APP_PROCESS_EXCLUSIONS.iter().map(|s| s.to_string()).collect();
        summary.extend(folders.iter().map(|p| p.to_string_lossy().to_string()));
        Ok(format!(
            "Windows Defender exclusions added: {}",
            summary.join(", ")
        ))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        crate::desk_log_error!("antivirus", "Failed to apply Windows Defender exclusions via PowerShell: {}", stderr);
        Err(format!(
            "Windows Defender refused the exclusion ({}). You can add the folders manually in Windows Security.",
            if stderr.is_empty() { "unknown error".to_string() } else { stderr }
        ))
    }
}

/// Open the Windows Security "Virus & threat protection" page so the user can
/// add the exclusion manually.
#[tauri::command]
pub fn open_windows_security() {
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", "windowsdefender://threatsettings"])
        .spawn();
}

/// Open the app install folder in Explorer (handy for manual exclusions).
#[tauri::command]
pub fn open_app_folder() {
    let path = LocalAppPaths::install_root();
    let _ = std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn();
}

/// Configure a `Command` to hide its console window on Windows.
#[cfg(target_os = "windows")]
fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

/// No-op on non-Windows platforms.
#[cfg(not(target_os = "windows"))]
fn hide_window(_command: &mut Command) {}
