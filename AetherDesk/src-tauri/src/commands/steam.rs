use crate::util::validation::validate_steam_path;
use crate::updater::dll::DllInstaller;
use crate::core::settings::SettingsManager;
use crate::steam::update_guard::SteamUpdateGuard;

#[tauri::command]
pub fn restart_steam(app: tauri::AppHandle) -> Result<(), String> {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes();

    let mut terminated = false;
    for process in sys.processes().values() {
        let name = process.name().to_lowercase();
        if name == "steam.exe" || name == "steam" {
            let _ = process.kill();
            terminated = true;
        }
    }

    if terminated {
        std::thread::sleep(std::time::Duration::from_millis(600));
    }

    let settings = SettingsManager::new(&app).load();
    let steam_dir = std::path::PathBuf::from(&settings.steam_path);

    if !steam_dir.exists() {
        return Err("Steam installation path does not exist. Please check your settings.".to_string());
    }

    let steam_exe = steam_dir.join("steam.exe");
    if !steam_exe.exists() {
        return Err(format!("steam.exe was not found in Steam directory: {:?}", steam_exe));
    }

    let mut cmd = std::process::Command::new(&steam_exe);
    cmd.current_dir(&steam_dir);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    cmd.spawn().map_err(|e| format!("Failed to launch Steam process: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn is_dll_installed(steam_path: String) -> Result<bool, String> {
    if steam_path.trim().is_empty() {
        return Ok(false);
    }
    Ok(DllInstaller::new(steam_path).verify_installation())
}

#[tauri::command]
pub fn is_steam_blocked(steam_path: String) -> Result<bool, String> {
    if steam_path.trim().is_empty() {
        return Ok(false);
    }
    SteamUpdateGuard::new(steam_path).is_blocked()
}

#[tauri::command]
pub fn block_steam_updates(steam_path: String) -> Result<String, String> {
    validate_steam_path(&steam_path)?;
    SteamUpdateGuard::new(steam_path).block_updates()?;
    Ok("Steam updates are now blocked.".to_string())
}

#[tauri::command]
pub fn unblock_steam_updates(steam_path: String) -> Result<String, String> {
    validate_steam_path(&steam_path)?;
    SteamUpdateGuard::new(steam_path).unblock_updates()?;
    Ok("Steam updates are now unblocked.".to_string())
}

