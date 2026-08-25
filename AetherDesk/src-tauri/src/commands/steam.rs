use crate::util::validation::validate_steam_path;
use crate::updater::dll::DllInstaller;
use crate::core::settings::SettingsManager;
use crate::steam::launch_options;
use crate::steam::update_guard::SteamUpdateGuard;
use std::path::Path;

/// Argomento di avvio che Aether usa per attivare il suo onlinefix per un gioco.
const AETHER_ONLINEFIX_TOKEN: &str = "-onlinefix";

/// LEGACY: argomento di avvio con cui le vecchie build attivavano la presenza
/// "sta giocando a" (presenza server-side Spacewar/480 + nome reale via
/// game_extra_info; il processo resta registrato con l'appid reale). Oggi
/// AetherDesk attiva la stessa sessione tramite `[presence] showonline_apps`
/// in aethercore.toml — niente sulla riga di comando del gioco (alcuni giochi
/// crashano su qualunque argomento extra: Selene ~Apoptosis~, Z.A.T.O.).
/// Il token resta riconosciuto/rimosso qui per migrare le configurazioni
/// esistenti.
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
        crate::core::steam_monitor::mark(false);
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
    // Kill+spawn riusciti: anticipa lo stato per la UI (il poller del monitor
    // corregge alla prossima scansione se lo spawn fallisse a valle).
    crate::core::steam_monitor::mark(true);
    Ok(())
}

#[tauri::command]
pub fn is_dll_installed(steam_path: String) -> Result<bool, String> {
    if steam_path.trim().is_empty() {
        return Ok(false);
    }
    Ok(DllInstaller::new(steam_path).verify_installation())
}

/// Stato "Steam in esecuzione" letto dal monitor condiviso (core::steam_monitor):
/// O(1), nessuna scansione di processo qui dentro. La UI sottoscrive anche
/// l'evento `steam://runtime-state` per aggiornarsi in tempo reale.
#[tauri::command]
pub fn is_steam_running() -> bool {
    crate::core::steam_monitor::is_steam_running()
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

/// True quando il gioco è in modalità onlinefix (masking 480 + integrazione OF).
///
/// Sorgente di verità: `[presence] onlinefix_apps` in aethercore.toml
/// (docs/05 §12). LEGACY: il token `-onlinefix` nelle LaunchOptions di Steam
/// vale ancora finché un set non lo migra.
#[tauri::command]
pub fn get_aether_onlinefix(app: tauri::AppHandle, app_id: u32) -> Result<bool, String> {
    for path in aethercore_toml_paths(&app) {
        if let Some(apps) = read_mode_apps(&path, PresenceMode::OnlineFix) {
            if apps.contains(&app_id) {
                return Ok(true);
            }
        }
    }
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

/// Attiva/disattiva la modalità onlinefix per il gioco SENZA scrivere nulla
/// sulla riga di comando: il marker è `[presence] onlinefix_apps` in
/// aethercore.toml (docs/05 §12). Attivando, l'app esce dalle altre liste
/// (mutua esclusione: il masking 480 è un superset della sola presenza).
/// I token `-onlinefix`/`-showonline` residui nelle LaunchOptions vengono
/// sempre rimossi (migrazione), preservando gli altri argomenti dell'utente.
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

    let marker_active = aethercore_toml_paths(&app)
        .iter()
        .any(|p| read_mode_apps(p, PresenceMode::OnlineFix).map(|apps| apps.contains(&app_id)).unwrap_or(false));
    let current = launch_options::get_launch_options(Path::new(&steam_path), app_id)?;
    let legacy_active = launch_options::has_launch_token(&current, AETHER_ONLINEFIX_TOKEN)
        || launch_options::has_launch_token(&current, AETHER_SHOWONLINE_TOKEN);
    if marker_active == enabled && !legacy_active {
        return Ok(if enabled {
            format!("Aether onlinefix is already enabled for app {app_id}.")
        } else {
            format!("Aether onlinefix is already disabled for app {app_id}.")
        });
    }

    // Migrazione: MAI più token Aether nelle LaunchOptions (crash class §11).
    let mut updated = launch_options::toggle_launch_token(&current, AETHER_ONLINEFIX_TOKEN, false);
    updated = launch_options::toggle_launch_token(&updated, AETHER_SHOWONLINE_TOKEN, false);
    if updated != current {
        launch_options::set_launch_options(Path::new(&steam_path), app_id, &updated)?;
    }

    let choice = if enabled { Some(PresenceMode::OnlineFix) } else { None };
    let mut touched = false;
    for path in aethercore_toml_paths(&app) {
        if update_mode_in_toml(&path, app_id, choice) {
            touched = true;
        }
    }

    crate::desk_log_info!(
        "steam",
        "Aether onlinefix {} for app {} (marker toml={} legacy_tokens_found={})",
        if enabled { "enabled" } else { "disabled" },
        app_id,
        touched,
        legacy_active
    );
    Ok(if enabled {
        format!("Aether onlinefix enabled for app {app_id} (no launch argument written; the game will be masked as Spacewar).")
    } else {
        format!("Aether onlinefix disabled for app {app_id}.")
    })
}

/// True quando il gioco ha un marker showonline attivo per Aether.
///
/// Sorgente di verità: `[presence] showonline_apps` in aethercore.toml (docs/05
/// §11). LEGACY: i token `-showonline` nelle LaunchOptions di Steam (build
/// precedenti) valgono ancora — la DLL continua a consumarli — ma ai prossimi
/// set AetherDesk li migra/rimuove perché alcuni giochi crashano su qualunque
/// argomento extra in argv (Selene ~Apoptosis~, Z.A.T.O.).
#[tauri::command]
pub fn get_aether_showonline(app: tauri::AppHandle, app_id: u32) -> Result<bool, String> {
    // Il marker nel toml è la fonte di verità e non dipende da steam_path.
    for path in aethercore_toml_paths(&app) {
        if let Some(apps) = read_mode_apps(&path, PresenceMode::ShowOnline) {
            if apps.contains(&app_id) {
                return Ok(true);
            }
        }
    }
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

/// Attiva/disattiva la presenza "-showonline" per il gioco SENZA scrivere
/// nulla sulla riga di comando del gioco: il marker è `showonline_apps` in
/// aethercore.toml (entrambe le copie gestite da AetherDesk). La DLL lo
/// rilegge ad ogni SpawnProcess — nessun riavvio di Steam necessario.
/// Eventuali token legacy in LaunchOptions vengono rimossi; attivando il
/// marker si rimuove anche `-onlinefix` (mutua esclusione): -showonline è
/// pensato per giochi singleplayer — niente masking del processo, solo la
/// presenza "sta giocando a" verso gli amici.
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

    let marker_active = aethercore_toml_paths(&app)
        .iter()
        .any(|p| read_mode_apps(p, PresenceMode::ShowOnline).map(|apps| apps.contains(&app_id)).unwrap_or(false));
    let current = launch_options::get_launch_options(Path::new(&steam_path), app_id)?;
    let legacy_active = launch_options::has_launch_token(&current, AETHER_SHOWONLINE_TOKEN)
        || (enabled && launch_options::has_launch_token(&current, AETHER_ONLINEFIX_TOKEN));
    if marker_active == enabled && !legacy_active
    {
        return Ok(if enabled {
            format!("Aether showonline is already enabled for app {app_id}.")
        } else {
            format!("Aether showonline is already disabled for app {app_id}.")
        });
    }

    // 1) Launch options: MAI -showonline (crash class argv/lauch-option,
    //    docs/05 §11). Eventuali token legacy vengono migrati: rimossi qui,
    //    sostituiti dal marker nel toml. Attivando, via anche -onlinefix.
    let mut updated = launch_options::toggle_launch_token(&current, AETHER_SHOWONLINE_TOKEN, false);
    if enabled {
        updated = launch_options::toggle_launch_token(&updated, AETHER_ONLINEFIX_TOKEN, false);
    }
    if updated != current {
        launch_options::set_launch_options(Path::new(&steam_path), app_id, &updated)?;
    }

    // 2) Marker negli array [presence] in TUTTE le copie di aethercore.toml
    //    esistenti: l'app esce da ogni lista e rientra solo in quella scelta.
    let choice = if enabled { Some(PresenceMode::ShowOnline) } else { None };
    let mut touched = false;
    for path in aethercore_toml_paths(&app) {
        if update_mode_in_toml(&path, app_id, choice) {
            touched = true;
        }
    }

    crate::desk_log_info!(
        "steam",
        "Aether showonline {} for app {} (marker toml={} legacy_token_removed={})",
        if enabled { "enabled" } else { "disabled" },
        app_id,
        touched,
        legacy_active
    );
    Ok(if enabled {
        format!("Aether showonline enabled for app {app_id} (no launch argument written; friends will see what you're playing).")
    } else {
        format!("Aether showonline disabled for app {app_id}.")
    })
}

use crate::core::presence_config::{
    PresenceMode, aethercore_toml_paths, read_default_mode, read_mode_apps,
    set_default_mode_in_toml, update_mode_in_toml,
};

/// True quando il gioco è in exclude_apps (hard opt-out: la DLL ignora
/// completamente il gioco, anche con token residui in argv).
#[tauri::command]
pub fn get_aether_excluded(app: tauri::AppHandle, app_id: u32) -> Result<bool, String> {
    Ok(aethercore_toml_paths(&app).iter().any(|p| {
        read_mode_apps(p, PresenceMode::Excluded)
            .map(|apps| apps.contains(&app_id))
            .unwrap_or(false)
    }))
}

/// Mette/toglie l'app dalla lista exclude_apps (ignorata interamente da
/// Aether: niente presenza, niente onlinefix, nessun token in argv).
#[tauri::command]
pub fn set_aether_excluded(
    app: tauri::AppHandle,
    app_id: u32,
    enabled: bool,
) -> Result<String, String> {
    let steam_path = SettingsManager::new(&app).load().steam_path;
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required.".to_string());
    }
    // Sicurezza: eventuali token Aether residui spariscono comunque.
    let current = launch_options::get_launch_options(Path::new(&steam_path), app_id)?;
    let mut updated = launch_options::toggle_launch_token(&current, AETHER_ONLINEFIX_TOKEN, false);
    updated = launch_options::toggle_launch_token(&updated, AETHER_SHOWONLINE_TOKEN, false);
    if updated != current {
        launch_options::set_launch_options(Path::new(&steam_path), app_id, &updated)?;
    }

    let choice = if enabled { Some(PresenceMode::Excluded) } else { None };
    for path in aethercore_toml_paths(&app) {
        update_mode_in_toml(&path, app_id, choice);
    }
    crate::desk_log_info!(
        "steam",
        "Aether exclude {} for app {}",
        if enabled { "enabled" } else { "disabled" },
        app_id
    );
    Ok(if enabled {
        format!("App {app_id} excluded from Aether (hard opt-out; nothing is written to its command line).")
    } else {
        format!("App {app_id} no longer excluded.")
    })
}

/// True quando la policy di default è `default_mode = "showonline"`: ogni
/// gioco presenta agli amici "sta giocando a" senza configurazione per-gioco.
#[tauri::command]
pub fn get_presence_default_mode(app: tauri::AppHandle) -> Result<bool, String> {
    for path in aethercore_toml_paths(&app) {
        if let Some(v) = read_default_mode(&path) {
            return Ok(v);
        }
    }
    Ok(false)
}

/// Imposta la policy di default (docs/05 §12): "showonline" rende la presenza
/// la norma per ogni app non elencata; gli array restano override espliciti.
#[tauri::command]
pub fn set_presence_default_mode(app: tauri::AppHandle, showonline: bool) -> Result<String, String> {
    for path in aethercore_toml_paths(&app) {
        set_default_mode_in_toml(&path, showonline);
    }
    crate::desk_log_info!(
        "steam",
        "presence default_mode = {}",
        if showonline { "showonline" } else { "none" }
    );
    Ok(if showonline {
        "Default mode is now showonline: every game (unless excluded/onlinefix) broadcasts what you're playing.".to_string()
    } else {
        "Default mode is now none: only apps explicitly listed get Aether presence.".to_string()
    })
}
