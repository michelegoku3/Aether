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
        if let Some(apps) = read_showonline_apps(&path) {
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
        .any(|p| read_showonline_apps(p).map(|apps| apps.contains(&app_id)).unwrap_or(false));
    let current = launch_options::get_launch_options(Path::new(&steam_path), app_id)?;
    let legacy_active = launch_options::has_launch_token(&current, AETHER_SHOWONLINE_TOKEN);
    if marker_active == enabled && !legacy_active
        && (!enabled || !launch_options::has_launch_token(&current, AETHER_ONLINEFIX_TOKEN))
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

    // 2) Marker showonline_apps in TUTTE le copie di aethercore.toml esistenti.
    let mut touched = false;
    for path in aethercore_toml_paths(&app) {
        if update_showonline_app_in_toml(&path, app_id, enabled) {
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

/// Le copie di aethercore.toml aggiornate da AetherDesk (stesso dual-path
/// già usato per custom_game_name in commands/settings.rs).
fn aethercore_toml_paths(app: &tauri::AppHandle) -> Vec<std::path::PathBuf> {
    let mut paths = vec![crate::core::paths::LocalAppPaths::config_dir().join("aethercore.toml")];
    let steam_path = SettingsManager::new(app).load().steam_path;
    if !steam_path.trim().is_empty() {
        paths.push(std::path::PathBuf::from(&steam_path).join("aethercore").join("aethercore.toml"));
    }
    paths
}

/// Estrae la lista appid da una riga `showonline_apps = [...]`. Riga
/// commentata (# …) → None (non è una fonte attiva).
fn read_showonline_apps(path: &std::path::Path) -> Option<Vec<u32>> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("showonline_apps") {
            let rest = rest.trim_start();
            if let Some(list) = rest.strip_prefix('=') {
                return Some(parse_appid_list(list));
            }
        }
    }
    None
}

fn parse_appid_list(text: &str) -> Vec<u32> {
    text.trim()
        .trim_start_matches('[')
        .split_terminator(|c: char| c == ',' || c == ']')
        .filter_map(|tok| tok.trim().parse::<u32>().ok())
        .collect()
}

/// Aggiunge/rimuove app_id da `showonline_apps` nel toml indicato.
/// Ritorna true se il file è stato modificato. File mancanti sono saltati
/// (le copie vengono create altrove); sezione [presence] mancante → creata
/// in coda al file.
fn update_showonline_app_in_toml(path: &std::path::Path, app_id: u32, enabled: bool) -> bool {
    if !path.exists() {
        return false;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let mut key_idx: Option<usize> = None;
    let mut presence_hdr: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with("showonline_apps") && trimmed["showonline_apps".len()..].trim_start().starts_with('=') {
            key_idx = Some(i);
        } else if trimmed == "[presence]" {
            presence_hdr = Some(i);
        }
    }

    let mut apps: Vec<u32> = key_idx
        .map(|i| {
            let line = &lines[i];
            let after_eq = line.splitn(2, '=').nth(1).unwrap_or("");
            parse_appid_list(after_eq)
        })
        .unwrap_or_default();
    let already = apps.contains(&app_id);
    if enabled && !already {
        apps.push(app_id);
    } else if !enabled && already {
        apps.retain(|&a| a != app_id);
    } else if key_idx.is_some() {
        return false;  // lista già nello stato richiesto
    }
    apps.sort_unstable();
    let new_line = format!(
        "showonline_apps = [{}]",
        apps.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
    );

    if let Some(i) = key_idx {
        lines[i] = new_line;
    } else if let Some(hdr) = presence_hdr {
        lines.insert(hdr + 1, new_line);
    } else {
        if content.trim().is_empty() {
            lines.clear();
        }
        lines.push("[presence]".to_string());
        lines.push(new_line);
    }
    let mut out = lines.join("\n");
    if content.ends_with('\n') || out.ends_with(']') {
        out.push('\n');
    }
    std::fs::write(path, out).is_ok()
}

