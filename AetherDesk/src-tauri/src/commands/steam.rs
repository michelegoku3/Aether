use crate::util::validation::validate_steam_path;
use crate::updater::dll::DllInstaller;
use crate::core::settings::SettingsManager;
use crate::steam::launch_options;
use crate::steam::update_guard::SteamUpdateGuard;
use std::path::Path;

/// Argomento di avvio che Aether usa per attivare il suo onlinefix per un gioco.
const AETHER_ONLINEFIX_TOKEN: &str = "-onlinefix";

/// Argomento di avvio che Aether usa per mostrare agli amici il gioco
/// singleplayer a cui si sta giocando (presenza server-side Spacewar/480 +
/// nome reale via game_extra_info, nessuna funzionalità online/masking
/// locale: il processo resta registrato con l'appid reale).
const AETHER_SHOWONLINE_TOKEN: &str = "-showonline";

#[tauri::command]
pub fn restart_steam(app: tauri::AppHandle) -> Result<(), String> {
    crate::core::logger::reset_session_dedup();
    crate::desk_log_info!("lifecycle", "Steam restart requested. Resetting AetherDesk session deduplication set.");

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
    crate::desk_log_info!("steam", "Blocking Steam updates in directory '{}'", steam_path);
    SteamUpdateGuard::new(steam_path).block_updates()?;
    Ok("Steam updates are now blocked.".to_string())
}

#[tauri::command]
pub fn unblock_steam_updates(steam_path: String) -> Result<String, String> {
    validate_steam_path(&steam_path)?;
    crate::desk_log_info!("steam", "Unblocking Steam updates in directory '{}'", steam_path);
    SteamUpdateGuard::new(steam_path).unblock_updates()?;
    Ok("Steam updates are now unblocked.".to_string())
}

/// True quando il gioco ha il token `-onlinefix` nelle LaunchOptions di Steam.
#[tauri::command]
pub fn get_aether_onlinefix(app: tauri::AppHandle, app_id: u32) -> Result<bool, String> {
    let steam_path = SettingsManager::new(&app).load().steam_path;
    if steam_path.trim().is_empty() {
        return Ok(false);
    }
    match launch_options::get_launch_options(Path::new(&steam_path), app_id) {
        Ok(options) => Ok(launch_options::has_launch_token(&options, AETHER_ONLINEFIX_TOKEN)),
        // Nessuna localconfig ancora (Steam mai avviato): semplicemente non attivo.
        Err(e) if e.contains("not found") => Ok(false),
        Err(e) => Err(e),
    }
}

/// Aggiunge o rimuove `-onlinefix` dalle LaunchOptions di Steam per il gioco,
/// preservando gli altri argomenti già presenti. Quando viene attivato,
/// rimuove `-showonline`: i due tag sono mutuamente esclusivi (il masking 480
/// di -onlinefix è un superset funzionale della sola presenza di -showonline).
#[tauri::command]
pub fn set_aether_onlinefix(
    app: tauri::AppHandle,
    app_id: u32,
    enabled: bool,
) -> Result<String, String> {
    let steam_path = SettingsManager::new(&app).load().steam_path;
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required.".to_string());
    }

    let current = launch_options::get_launch_options(Path::new(&steam_path), app_id)?;
    let mut updated = launch_options::toggle_launch_token(&current, AETHER_ONLINEFIX_TOKEN, enabled);
    if enabled {
        updated = launch_options::toggle_launch_token(&updated, AETHER_SHOWONLINE_TOKEN, false);
    }
    if updated == current {
        return Ok(if enabled {
            format!("Aether onlinefix is already enabled for app {app_id}.")
        } else {
            format!("Aether onlinefix is already disabled for app {app_id}.")
        });
    }

    launch_options::set_launch_options(Path::new(&steam_path), app_id, &updated)?;
    crate::desk_log_info!(
        "steam",
        "Aether onlinefix {} for app {} (launch options: '{}')",
        if enabled { "enabled" } else { "disabled" },
        app_id,
        updated
    );
    Ok(if enabled {
        format!("Aether onlinefix enabled for app {app_id} (-onlinefix added to launch options).")
    } else {
        format!("Aether onlinefix disabled for app {app_id} (-onlinefix removed from launch options).")
    })
}

/// True quando il gioco ha il token `-showonline` nelle LaunchOptions di Steam.
#[tauri::command]
pub fn get_aether_showonline(app: tauri::AppHandle, app_id: u32) -> Result<bool, String> {
    let steam_path = SettingsManager::new(&app).load().steam_path;
    if steam_path.trim().is_empty() {
        return Ok(false);
    }
    match launch_options::get_launch_options(Path::new(&steam_path), app_id) {
        Ok(options) => Ok(launch_options::has_launch_token(&options, AETHER_SHOWONLINE_TOKEN)),
        // Nessuna localconfig ancora (Steam mai avviato): semplicemente non attivo.
        Err(e) if e.contains("not found") => Ok(false),
        Err(e) => Err(e),
    }
}

/// Aggiunge o rimuove `-showonline` dalle LaunchOptions di Steam per il gioco.
/// Quando viene attivato, rimuove `-onlinefix` (mutua esclusione): -showonline
/// è pensato per giochi singleplayer — niente masking del processo, nessuna
/// funzionalità online; solo la presenza "sta giocando a" verso gli amici.
#[tauri::command]
pub fn set_aether_showonline(
    app: tauri::AppHandle,
    app_id: u32,
    enabled: bool,
) -> Result<String, String> {
    let steam_path = SettingsManager::new(&app).load().steam_path;
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required.".to_string());
    }

    let current = launch_options::get_launch_options(Path::new(&steam_path), app_id)?;
    let mut updated = launch_options::toggle_launch_token(&current, AETHER_SHOWONLINE_TOKEN, enabled);
    if enabled {
        updated = launch_options::toggle_launch_token(&updated, AETHER_ONLINEFIX_TOKEN, false);
    }
    if updated == current {
        return Ok(if enabled {
            format!("Aether showonline is already enabled for app {app_id}.")
        } else {
            format!("Aether showonline is already disabled for app {app_id}.")
        });
    }

    launch_options::set_launch_options(Path::new(&steam_path), app_id, &updated)?;
    crate::desk_log_info!(
        "steam",
        "Aether showonline {} for app {} (launch options: '{}')",
        if enabled { "enabled" } else { "disabled" },
        app_id,
        updated
    );
    Ok(if enabled {
        format!("Aether showonline enabled for app {app_id} (-showonline added to launch options; friends will see what you're playing).")
    } else {
        format!("Aether showonline disabled for app {app_id} (-showonline removed from launch options).")
    })
}

