// Windows Defender exclusion helpers.
//
// AetherDesk applies crack files into Steam library folders, which Windows
// Defender may flag as false positives and silently quarantine. The exclusion
// set therefore covers:
//
//   1. The AetherDesk install folder (the executable and its data).
//   2. Every Steam library folder discovered from the configured Steam path
//      (main install + any extra libraries registered in libraryfolders.vdf).
//
// Because the app runs with admin rights, exclusions are applied via
// PowerShell (`Add-MpPreference`) with a hidden console window.
use crate::local_app_paths::LocalAppPaths;
use crate::settings::SettingsManager;
use crate::steam_library::SteamLibraryScanner;
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
/// Returns a de-duplicated list containing:
///   - The AetherDesk install root.
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

/// Try to add all relevant folders to Windows Defender exclusions via
/// PowerShell (hidden window, admin rights already present). On success the
/// prompt is marked as done.
#[tauri::command]
pub fn apply_antivirus_exclusion(app: tauri::AppHandle) -> Result<String, String> {
    let folders = collect_exclusion_paths(&app);

    if folders.is_empty() {
        return Err(
            "No valid folders found to exclude. Please configure your Steam path in Settings first."
                .to_string(),
        );
    }

    // Build a single PowerShell script that adds every path in one invocation,
    // so the user sees only one UAC/PowerShell flash instead of N.
    let script = folders
        .iter()
        .map(|path| {
            let escaped = path.to_string_lossy().replace('\'', "''");
            format!("Add-MpPreference -ExclusionPath '{}' -ErrorAction Stop", escaped)
        })
        .collect::<Vec<_>>()
        .join("; ");

    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script]);
    hide_window(&mut command);

    let output = command
        .output()
        .map_err(|error| format!("Failed to launch PowerShell: {}", error))?;

    if output.status.success() {
        acknowledge_antivirus_exclusion(app)?;
        let summary = folders
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        Ok(format!(
            "Added {} folder(s) to Windows Defender exclusions: {}",
            folders.len(),
            summary
        ))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
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
