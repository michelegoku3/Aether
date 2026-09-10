use crate::util::validation::validate_steam_path;
use crate::updater::dll::DllInstaller;
use crate::core::settings::SettingsManager;
use crate::steam::launch_options;
use crate::steam::resolve::{normalize_steam_path, resolve_steam_path};
use crate::steam::update_guard::SteamUpdateGuard;
use crate::util::dialog::file_path_to_string;
use std::path::{Path, PathBuf};
use tauri_plugin_dialog::DialogExt;

/// Argomento di avvio che Aether usa per attivare la sua modalità AetherOnline
/// (masking 480 + payload) per un gioco. Nome scelto per non confondersi con
/// la crack online-fix.me (OFME).
const AETHERONLINE_TOKEN: &str = "-aetheronline";

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
pub fn start_steam(app: tauri::AppHandle) -> Result<String, String> {
    crate::core::logger::reset_session_dedup();
    let settings = SettingsManager::new(&app).load();
    let steam_dir = std::path::PathBuf::from(&settings.steam_path);

    // Start = SOLO spawn: se Steam è già in esecuzione (stato del monitor,
    // O(1)) è un no-op documentato — non uccide mai nulla.
    if crate::core::steam_monitor::is_steam_running() {
        crate::desk_log_info!("lifecycle", "start_steam: Steam already running, no-op");
        crate::core::steam_monitor::mark(true);
        return Ok("Steam is already running.".to_string());
    }

    crate::desk_log_info!("lifecycle", "start_steam: launching Steam from {}", steam_dir.display());
    let exe = crate::core::steam_process::spawn_steam(&steam_dir)?;
    crate::core::steam_monitor::mark(true);
    crate::desk_log_info!("lifecycle", "start_steam: spawned {}", exe.display());
    Ok("Steam is starting.".to_string())
}

#[tauri::command]
pub fn restart_steam(app: tauri::AppHandle) -> Result<String, String> {
    crate::core::logger::reset_session_dedup();
    crate::desk_log_info!("lifecycle", "restart_steam: requested. Resetting AetherDesk session deduplication set.");

    let settings = SettingsManager::new(&app).load();
    let steam_dir = std::path::PathBuf::from(&settings.steam_path);

    if crate::core::steam_process::kill_steam() {
        crate::core::steam_monitor::mark(false);
        // Attesa vera: lo spawn avviene SOLO dopo che il processo è uscito,
        // altrimenti un secondo steam.exe entra in race con il single-instance
        // dell'istanza in uscita (il classico "restart che non funziona").
        let gone = crate::core::steam_process::wait_steam_gone();
        if !gone {
            crate::desk_log_warn!("lifecycle", "restart_steam: Steam still present after kill, proceeding anyway");
        } else {
            crate::desk_log_info!("lifecycle", "restart_steam: Steam exited, relaunching");
        }
    } else {
        crate::desk_log_info!("lifecycle", "restart_steam: Steam was not running, just starting");
    }

    let exe = crate::core::steam_process::spawn_steam(&steam_dir)?;
    // Spawn riuscito: anticipa lo stato per la UI (il poller del monitor
    // corregge alla prossima scansione se lo spawn fallisse a valle).
    crate::core::steam_monitor::mark(true);
    crate::desk_log_info!("lifecycle", "restart_steam: spawned {}", exe.display());
    Ok("Steam is restarting.".to_string())
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

/// True quando il gioco è in modalità AetherOnline (masking 480 + payload).
///
/// Sorgente di verità: `[presence] aetheronline_apps` in aethercore.toml
/// (docs/05 §12). LEGACY: il token `-aetheronline` nelle LaunchOptions di
/// Steam vale ancora finché un set non lo migra.
#[tauri::command]
pub fn get_aetheronline(app: tauri::AppHandle, app_id: u32) -> Result<bool, String> {
    // exclude vince sul token argv residuo: altrimenti il popup mostra
    // Online Aether ACTIVE insieme a UCO2.
    if aethercore_toml_paths(&app).iter().any(|p| {
        read_mode_apps(p, PresenceMode::Excluded)
            .map(|apps| apps.contains(&app_id))
            .unwrap_or(false)
    }) {
        return Ok(false);
    }
    for path in aethercore_toml_paths(&app) {
        if let Some(apps) = read_mode_apps(&path, PresenceMode::AetherOnline) {
            if apps.contains(&app_id) {
                return Ok(true);
            }
        }
    }
    let steam_path = SettingsManager::new(&app).load().steam_path;
    // Unreachable Steam means no legacy token can be active either.
    if resolve_steam_path(&steam_path).is_err() {
        return Ok(false);
    }
    match launch_options::get_launch_options(Path::new(&steam_path), app_id) {
        Ok(options) => Ok(launch_options::has_launch_token(&options, AETHERONLINE_TOKEN)),
        // Nessuna localconfig ancora (Steam mai avviato): semplicemente non attivo.
        Err(e) if e.contains("not found") => Ok(false),
        Err(e) => Err(e),
    }
}

/// Attiva/disattiva la modalità AetherOnline per il gioco SENZA scrivere nulla
/// sulla riga di comando: il marker è `[presence] aetheronline_apps` in
/// aethercore.toml (docs/05 §12). Attivando, l'app esce dalle altre liste
/// (mutua esclusione: il masking 480 è un superset della sola presenza).
/// I token `-aetheronline`/`-showonline` residui nelle LaunchOptions
/// vengono sempre rimossi (migrazione), preservando gli altri argomenti
/// dell'utente.
#[tauri::command]
pub fn set_aetheronline(
    app: tauri::AppHandle,
    app_id: u32,
    enabled: bool,
) -> Result<String, String> {
    let steam_path = SettingsManager::new(&app).load().steam_path;
    validate_steam_path(&steam_path)?;

    if enabled {
        let foreign = crate::commands::online::foreign_for_app(&app, app_id);
        if foreign.ofme {
            return Err(foreign.refuse_online_aether());
        }
        if crate::commands::online::uco2_enabled(app_id) || foreign.uco2 {
            return Err(if foreign.uco2 && !crate::commands::online::uco2_enabled(app_id) {
                foreign.refuse_online_aether_uco2()
            } else {
                "Disable UCO2 first — Online Aether and UCO2 cannot share a process.".to_string()
            });
        }
    }

    let marker_active = aethercore_toml_paths(&app)
        .iter()
        .any(|p| read_mode_apps(p, PresenceMode::AetherOnline).map(|apps| apps.contains(&app_id)).unwrap_or(false));
    let current = launch_options::get_launch_options(Path::new(&steam_path), app_id)?;
    let legacy_active = launch_options::has_launch_token(&current, AETHERONLINE_TOKEN)
        || launch_options::has_launch_token(&current, AETHER_SHOWONLINE_TOKEN);
    if marker_active == enabled && !legacy_active {
        return Ok(if enabled {
            format!("AetherOnline is already enabled for app {app_id}.")
        } else {
            format!("AetherOnline is already disabled for app {app_id}.")
        });
    }

    // Migrazione: MAI più token Aether nelle LaunchOptions (crash class §11).
    let mut updated = launch_options::toggle_launch_token(&current, AETHERONLINE_TOKEN, false);
    updated = launch_options::toggle_launch_token(&updated, AETHER_SHOWONLINE_TOKEN, false);
    if updated != current {
        launch_options::set_launch_options(Path::new(&steam_path), app_id, &updated)?;
    }

    let choice = if enabled { Some(PresenceMode::AetherOnline) } else { None };
    let mut touched = false;
    for path in aethercore_toml_paths(&app) {
        if update_mode_in_toml(&path, app_id, choice) {
            touched = true;
        }
    }

    crate::desk_log_info!(
        "steam",
        "AetherOnline {} for app {} (marker toml={} legacy_tokens_found={})",
        if enabled { "enabled" } else { "disabled" },
        app_id,
        touched,
        legacy_active
    );
    Ok(if enabled {
        format!("AetherOnline enabled for app {app_id} (no launch argument written; the game will be masked as Spacewar).")
    } else {
        format!("AetherOnline disabled for app {app_id}.")
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
    // Unreachable Steam means no legacy token can be active either.
    if resolve_steam_path(&steam_path).is_err() {
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
/// marker si rimuove anche `-aetheronline` (mutua esclusione): -showonline è
/// pensato per giochi singleplayer — niente masking del processo, solo la
/// presenza "sta giocando a" verso gli amici.
#[tauri::command]
pub fn set_aether_showonline(
    app: tauri::AppHandle,
    app_id: u32,
    enabled: bool,
) -> Result<String, String> {
    let steam_path = SettingsManager::new(&app).load().steam_path;
    validate_steam_path(&steam_path)?;

    if enabled {
        let foreign = crate::commands::online::foreign_for_app(&app, app_id);
        if foreign.ofme {
            return Err(foreign.refuse_showonline());
        }
        if crate::commands::online::uco2_enabled(app_id) || foreign.uco2 {
            return Err(if foreign.uco2 {
                foreign.refuse_showonline_uco2()
            } else {
                "Disable UCO2 first — Show Online remaps presence to Spacewar and breaks UCO2 invites.".to_string()
            });
        }
    }

    let marker_active = aethercore_toml_paths(&app)
        .iter()
        .any(|p| read_mode_apps(p, PresenceMode::ShowOnline).map(|apps| apps.contains(&app_id)).unwrap_or(false));
    let current = launch_options::get_launch_options(Path::new(&steam_path), app_id)?;
    let legacy_active = launch_options::has_launch_token(&current, AETHER_SHOWONLINE_TOKEN)
        || (enabled && launch_options::has_launch_token(&current, AETHERONLINE_TOKEN));
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
    //    sostituiti dal marker nel toml. Attivando, via anche -aetheronline.
    let mut updated = launch_options::toggle_launch_token(&current, AETHER_SHOWONLINE_TOKEN, false);
    if enabled {
        updated = launch_options::toggle_launch_token(&updated, AETHERONLINE_TOKEN, false);
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
/// Aether: niente presenza, niente aetheronline, nessun token in argv).
#[tauri::command]
pub fn set_aether_excluded(
    app: tauri::AppHandle,
    app_id: u32,
    enabled: bool,
) -> Result<String, String> {
    let steam_path = SettingsManager::new(&app).load().steam_path;
    // Token residui: best-effort. L'exclude deve comunque scriversi.
    if !steam_path.trim().is_empty() {
        if let Ok(current) = launch_options::get_launch_options(Path::new(&steam_path), app_id) {
            let mut updated = launch_options::toggle_launch_token(&current, AETHERONLINE_TOKEN, false);
            updated = launch_options::toggle_launch_token(&updated, AETHER_SHOWONLINE_TOKEN, false);
            if updated != current {
                let _ = launch_options::set_launch_options(Path::new(&steam_path), app_id, &updated);
            }
        }
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
        "Default mode is now showonline: every game (unless excluded/aetheronline) broadcasts what you're playing.".to_string()
    } else {
        "Default mode is now none: only apps explicitly listed get Aether presence.".to_string()
    })
}

/// Persist that the user has pressed "I understand" on the OST pattern-source
/// warning, so the popup shows only on first enable.
#[tauri::command]
pub fn acknowledge_ost_warning(app: tauri::AppHandle) -> Result<(), String> {
    let manager = crate::core::settings::SettingsManager::new(&app);
    let mut settings = manager.load();
    settings.ost_warning_acknowledged = true;
    manager.save(&settings)
}

/// True when the OST pattern source (OpenSteam001/steam-monitor) is enabled
/// via `[network] use_ost_source`. Missing key => false (opt-in OFF).
#[tauri::command]
pub fn get_ost_source_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    for path in crate::core::ost_config::aethercore_toml_paths(&app) {
        if let Some(v) = crate::core::ost_config::read_ost_enabled(&path) {
            return Ok(v);
        }
    }
    Ok(false)
}

/// Enables/disables the OST pattern source fallback. Applies to the next
/// pattern download (the DLL hot-reloads `aethercore.toml` on every game
/// launch): no Steam restart needed.
#[tauri::command]
pub fn set_ost_source_enabled(app: tauri::AppHandle, enabled: bool) -> Result<String, String> {
    for path in crate::core::ost_config::aethercore_toml_paths(&app) {
        crate::core::ost_config::set_ost_enabled_in_toml(&path, enabled);
    }
    crate::desk_log_info!(
        "steam",
        "Pattern OST source {}",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(if enabled {
        "OST pattern source enabled: OpenSteamTool is now used as a last-resort fallback.".to_string()
    } else {
        "OST pattern source disabled: only MigoReleases and KoriaPolis are used.".to_string()
    })
}

/// True when the missing `*.manifest` backups are recopied from
/// `AetherData\backup\<app_id>\lua` into `Steam\depotcache` on every Steam
/// start via `[manifest_cache] restore_on_startup`.
/// Missing key => true (default ON: uninstalling a game wipes depotcache,
/// and Steam no longer serves manifests without authentication).
#[tauri::command]
pub fn get_manifest_restore_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    for path in crate::core::manifest_restore_config::aethercore_toml_paths(&app) {
        if let Some(v) = crate::core::manifest_restore_config::read_manifest_restore_enabled(&path) {
            return Ok(v);
        }
    }
    Ok(true)
}

/// Enables/disables the startup refill of `Steam\depotcache` from the local
/// manifest backups. Applies on the next Steam start (the DLL runs the
/// restore once per Steam process, right after injection).
#[tauri::command]
pub fn set_manifest_restore_enabled(app: tauri::AppHandle, enabled: bool) -> Result<String, String> {
    for path in crate::core::manifest_restore_config::aethercore_toml_paths(&app) {
        crate::core::manifest_restore_config::set_manifest_restore_enabled_in_toml(&path, enabled);
    }
    crate::desk_log_info!(
        "steam",
        "Manifest restore on startup {}",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(if enabled {
        "Manifest restore enabled: missing .manifest backups will be copied back into Steam depotcache on every Steam start.".to_string()
    } else {
        "Manifest restore disabled: Steam depotcache will no longer be refilled from backups at startup.".to_string()
    })
}

// ---------------------------------------------------------------------------
// Steam installation path: picker, live validation, auto-detection.
// Thin Tauri wrappers over `steam::resolve` (which owns all the logic).
// ---------------------------------------------------------------------------

/// Open the native folder picker for the Steam installation directory.
/// Returns `None` when the user cancels. The dialog starts at the configured
/// path when usable, else at the auto-detected installation, else at the
/// default Program Files location.
#[tauri::command]
pub async fn pick_steam_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let start_dir = pick_steam_folder_start_dir(&app);
    crate::desk_log_info!(
        "steam",
        "Opening Steam folder picker (start dir: {})",
        start_dir.display()
    );

    let picked = app
        .dialog()
        .file()
        .set_title("Select your Steam installation folder (the one containing steam.exe)")
        .set_directory(&start_dir)
        .blocking_pick_folder();

    let selected = picked.and_then(file_path_to_string).map(|path| {
        // Persist the canonical form immediately at the source: the frontend
        // saves this value verbatim, and normalization is idempotent.
        normalize_steam_path(&path)
    });
    if let Some(path) = &selected {
        crate::desk_log_info!("steam", "Steam folder picked: '{}'", path);
    }
    Ok(selected.filter(|path| !path.is_empty()))
}

fn pick_steam_folder_start_dir(app: &tauri::AppHandle) -> PathBuf {
    let configured = SettingsManager::new(app).load().steam_path;
    let normalized = normalize_steam_path(&configured);
    if !normalized.is_empty() {
        let dir = PathBuf::from(&normalized);
        // Accept the configured path itself, or its parent when the stored
        // value points at a file / no longer exists (dialog still opens near
        // the user's intent instead of a generic location).
        if dir.is_dir() {
            return dir;
        }
        if let Some(parent) = dir.parent().filter(|parent| parent.is_dir()) {
            return parent.to_path_buf();
        }
    }
    if let Some(detected) = crate::steam::resolve::detect_steam_path() {
        return detected;
    }
    PathBuf::from(crate::steam::resolve::LEGACY_DEFAULT_STEAM_PATH)
}

/// Live validation result for the Settings UI: never fails, always answers.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamPathCheck {
    pub valid: bool,
    pub normalized: String,
    pub error: Option<String>,
}

/// Validate an arbitrary Steam path string for live UI feedback.
/// Read-only: inspects the filesystem, writes nothing.
#[tauri::command]
pub fn check_steam_path(path: String) -> SteamPathCheck {
    match resolve_steam_path(&path) {
        Ok(resolved) => SteamPathCheck {
            valid: true,
            normalized: resolved.display().to_string(),
            error: None,
        },
        Err(error) => SteamPathCheck {
            valid: false,
            normalized: normalize_steam_path(&path),
            error: Some(error.message(&path)),
        },
    }
}

/// Best-effort Steam auto-detection (registry → running process →
/// well-known locations). Returns `None` when nothing is found.
#[tauri::command]
pub fn detect_steam_path() -> Option<String> {
    let found = crate::steam::resolve::detect_steam_installation();
    match &found {
        Some((path, source)) => crate::desk_log_info!(
            "steam",
            "Auto-detected Steam at {} (source={})",
            path.display(),
            source.as_log_label()
        ),
        None => crate::desk_log_warn!("steam", "Steam auto-detection found nothing"),
    }
    found.map(|(path, _)| path.display().to_string())
}
