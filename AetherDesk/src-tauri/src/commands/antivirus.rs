// Windows Defender exclusion helpers.
//
// AetherDesk (and the cracks it applies) can be flagged by Windows Defender as
// false positives. On first run after install, and after updates while the
// prompt has never been acknowledged, the frontend shows a modal that lets the
// user add the app install folder to Defender exclusions.
//
// Because the app is launched with admin rights, we can apply the exclusion via
// PowerShell (`Add-MpPreference`) directly. The PowerShell process is started
// with a hidden console window so the user never sees a cmd flash.
use crate::local_app_paths::LocalAppPaths;
use crate::settings::SettingsManager;
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

/// Try to add the app install folder to Windows Defender exclusions via
/// PowerShell (hidden window, admin rights already present). On success the
/// prompt is marked as done.
#[tauri::command]
pub fn apply_antivirus_exclusion(app: tauri::AppHandle) -> Result<String, String> {
    let install_dir = LocalAppPaths::install_root();
    let path = install_dir.to_string_lossy().to_string();

    let script = format!(
        "Add-MpPreference -ExclusionPath '{}' -ErrorAction Stop",
        path.replace('\'', "''")
    );

    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script]);
    // Hide the console window (the app is a GUI and already runs as admin).
    hide_window(&mut command);

    let output = command
        .output()
        .map_err(|error| format!("Failed to launch PowerShell: {}", error))?;

    if output.status.success() {
        // Mark as done so it is not asked again.
        acknowledge_antivirus_exclusion(app)?;
        Ok(format!(
            "AetherDesk folder added to Windows Defender exclusions: {}",
            path
        ))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!(
            "Windows Defender refused the exclusion ({}). You can add it manually in Windows Security.",
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
