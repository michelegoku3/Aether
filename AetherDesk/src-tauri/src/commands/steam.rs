use super::{command_steam_path, configured_steam_path};
use crate::core::settings::load_settings;
use crate::steam::resolve::{normalize_steam_path, resolve_steam_path};
use crate::steam::update_guard::SteamUpdateGuard;
use crate::updater::dll::DllInstaller;
use crate::util::dialog::file_path_to_string;
use crate::util::validation::validate_steam_path;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub fn start_steam(app: tauri::AppHandle) -> Result<String, String> {
    crate::core::logger::reset_session_dedup();
    let settings = load_settings(&app);
    let steam_dir = std::path::PathBuf::from(&settings.steam_path);

    // Start = spawn ONLY: when Steam is already running (monitor state,
    // O(1)) it is a documented no-op — it never kills anything.
    if crate::core::steam_monitor::is_steam_running() {
        crate::desk_log_info!("lifecycle", "start_steam: Steam already running, no-op");
        crate::core::steam_monitor::mark(true);
        return Ok("Steam is already running.".to_string());
    }

    crate::desk_log_info!(
        "lifecycle",
        "start_steam: launching Steam from {}",
        steam_dir.display()
    );
    let exe = crate::core::steam_process::spawn_steam(&steam_dir)?;
    crate::core::steam_monitor::mark(true);
    crate::desk_log_info!("lifecycle", "start_steam: spawned {}", exe.display());
    Ok("Steam is starting.".to_string())
}

#[tauri::command]
pub fn restart_steam(app: tauri::AppHandle) -> Result<String, String> {
    crate::core::logger::reset_session_dedup();
    crate::desk_log_info!(
        "lifecycle",
        "restart_steam: requested. Resetting AetherDesk session deduplication set."
    );

    let settings = load_settings(&app);
    let steam_dir = std::path::PathBuf::from(&settings.steam_path);

    if crate::core::steam_process::kill_steam() {
        crate::core::steam_monitor::mark(false);
        // Real wait: the spawn happens ONLY after the process has exited,
        // otherwise a second steam.exe races the single instance of the
        // exiting one (the classic "restart that does nothing").
        let gone = crate::core::steam_process::wait_steam_gone();
        if !gone {
            crate::desk_log_warn!(
                "lifecycle",
                "restart_steam: Steam still present after kill, proceeding anyway"
            );
        } else {
            crate::desk_log_info!("lifecycle", "restart_steam: Steam exited, relaunching");
        }
    } else {
        crate::desk_log_info!(
            "lifecycle",
            "restart_steam: Steam was not running, just starting"
        );
    }

    let exe = crate::core::steam_process::spawn_steam(&steam_dir)?;
    // Spawn succeeded: anticipate the state for the UI (the monitor poller
    // corrects it on the next scan if the downstream spawn failed).
    crate::core::steam_monitor::mark(true);
    crate::desk_log_info!("lifecycle", "restart_steam: spawned {}", exe.display());
    Ok("Steam is restarting.".to_string())
}

#[tauri::command]
pub fn is_dll_installed(app: tauri::AppHandle) -> Result<bool, String> {
    // State read: with no configured path the answer is "not installed",
    // not an error.
    let steam_path = configured_steam_path(&app).unwrap_or_default();
    if steam_path.trim().is_empty() {
        return Ok(false);
    }
    Ok(DllInstaller::new(&steam_path).verify_installation())
}

/// "Steam running" state read from the shared monitor (core::steam_monitor):
/// O(1), no process scan in here. The UI also subscribes to the
/// `steam://runtime-state` event for real-time updates.
#[tauri::command]
pub fn is_steam_running() -> bool {
    crate::core::steam_monitor::is_steam_running()
}

#[tauri::command]
pub fn is_steam_blocked(app: tauri::AppHandle) -> Result<bool, String> {
    let steam_path = configured_steam_path(&app).unwrap_or_default();
    if steam_path.trim().is_empty() {
        return Ok(false);
    }
    SteamUpdateGuard::new(steam_path).is_blocked()
}

#[tauri::command]
pub fn block_steam_updates(app: tauri::AppHandle) -> Result<String, String> {
    // Write into the Steam folder: strict validation, readable error.
    let steam_path = command_steam_path(&app)?;
    crate::desk_log_info!(
        "steam",
        "Blocking Steam updates in directory '{}'",
        steam_path
    );
    SteamUpdateGuard::new(steam_path).block_updates()?;
    Ok("Steam updates are now blocked.".to_string())
}

#[tauri::command]
pub fn unblock_steam_updates(app: tauri::AppHandle) -> Result<String, String> {
    let steam_path = command_steam_path(&app)?;
    crate::desk_log_info!(
        "steam",
        "Unblocking Steam updates in directory '{}'",
        steam_path
    );
    SteamUpdateGuard::new(steam_path).unblock_updates()?;
    Ok("Steam updates are now unblocked.".to_string())
}

// UTC `YYYYmmdd-HHmmss`, the only date code this crate needs: backup names sort
// chronologically and two repairs in the same second cannot silently collide.
fn backup_timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days, offset to the Unix epoch.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096).div_euclid(365);
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2).div_euclid(153);
    let day = doy - (153 * mp + 2).div_euclid(5) + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        of_day / 3_600,
        (of_day % 3_600) / 60,
        of_day % 60
    )
}

/// What the appinfo cache repair moved where.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfoRepair {
    pub backup_path: String,
    pub bytes: u64,
}

/// Moves Steam's `appcache/appinfo.vdf` aside so Steam rebuilds it.
///
/// The cache is what makes Steam list an app it has an AppInfo for; when it is
/// stuck behind a frozen change number, a fresh `.lua` app never appears in the
/// library. Steam rewrites the file on exit and at startup, so this refuses to
/// run while Steam is open: the rename must happen with the file quiescent.
#[tauri::command]
pub fn repair_appinfo_cache(app: tauri::AppHandle) -> Result<AppInfoRepair, String> {
    if crate::core::steam_monitor::is_steam_running() {
        return Err("Close Steam before repairing its metadata cache.".to_string());
    }
    let steam_path = command_steam_path(&app)?;
    let cache = PathBuf::from(&steam_path)
        .join("appcache")
        .join("appinfo.vdf");
    if !cache.is_file() {
        return Err(format!(
            "Steam's appinfo cache was not found at {}",
            cache.display()
        ));
    }
    let bytes = std::fs::metadata(&cache)
        .map_err(|error| format!("Cannot read {}: {error}", cache.display()))?
        .len();
    let backup = cache.with_file_name(format!("appinfo.vdf.bak-{}", backup_timestamp()));
    if backup.exists() {
        return Err(format!(
            "A backup already exists at {}; move it away and retry.",
            backup.display()
        ));
    }
    std::fs::rename(&cache, &backup).map_err(|error| {
        format!(
            "Cannot move {} to {}: {error}",
            cache.display(),
            backup.display()
        )
    })?;
    crate::desk_log_info!(
        "steam",
        "Moved Steam appinfo cache to '{}' ({} bytes); Steam rebuilds it on the next start.",
        backup.display(),
        bytes
    );
    Ok(AppInfoRepair {
        backup_path: backup.to_string_lossy().to_string(),
        bytes,
    })
}

/// True when the game is in AetherOnline mode (480 masking + payload).
///
/// Source of truth: `[presence] aetheronline_apps` in aethercore.toml.
#[tauri::command]
pub fn get_aetheronline(app: tauri::AppHandle, app_id: u32) -> Result<bool, String> {
    // exclude wins: otherwise the popup shows Online Aether ACTIVE together
    // with UCO2.
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
    Ok(false)
}

/// Enables/disables AetherOnline mode for the game: the marker is
/// `[presence] aetheronline_apps` in aethercore.toml. On enable, the app leaves
/// the other lists (mutual exclusion: 480 masking is a superset of presence
/// alone). Nothing is written to the game's command line.
#[tauri::command]
pub fn set_aetheronline(
    app: tauri::AppHandle,
    app_id: u32,
    enabled: bool,
) -> Result<String, String> {
    let steam_path = load_settings(&app).steam_path;
    validate_steam_path(&steam_path)?;

    if enabled {
        let foreign = crate::commands::online::foreign_for_app(&app, app_id);
        if foreign.ofme {
            return Err(foreign.refuse_online_aether());
        }
        if crate::commands::online::uco2_enabled(app_id) || foreign.uco2 {
            return Err(
                if foreign.uco2 && !crate::commands::online::uco2_enabled(app_id) {
                    foreign.refuse_online_aether_uco2()
                } else {
                    "Disable UCO2 first — Online Aether and UCO2 cannot share a process."
                        .to_string()
                },
            );
        }
    }

    let marker_active = aethercore_toml_paths(&app).iter().any(|p| {
        read_mode_apps(p, PresenceMode::AetherOnline)
            .map(|apps| apps.contains(&app_id))
            .unwrap_or(false)
    });
    if marker_active == enabled {
        return Ok(if enabled {
            format!("AetherOnline is already enabled for app {app_id}.")
        } else {
            format!("AetherOnline is already disabled for app {app_id}.")
        });
    }

    let choice = if enabled {
        Some(PresenceMode::AetherOnline)
    } else {
        None
    };
    let mut touched = false;
    for path in aethercore_toml_paths(&app) {
        if update_mode_in_toml(&path, app_id, choice)? {
            touched = true;
        }
    }

    crate::desk_log_info!(
        "steam",
        "AetherOnline {} for app {} (marker toml={})",
        if enabled { "enabled" } else { "disabled" },
        app_id,
        touched
    );
    Ok(if enabled {
        format!("AetherOnline enabled for app {app_id} (the game will be masked as Spacewar).")
    } else {
        format!("AetherOnline disabled for app {app_id}.")
    })
}

/// True when the game has an active showonline marker for Aether.
///
/// Source of truth: `[presence] showonline_apps` in aethercore.toml.
#[tauri::command]
pub fn get_aether_showonline(app: tauri::AppHandle, app_id: u32) -> Result<bool, String> {
    for path in aethercore_toml_paths(&app) {
        if let Some(apps) = read_mode_apps(&path, PresenceMode::ShowOnline) {
            if apps.contains(&app_id) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Enables/disables the showonline presence for the game: the marker is
/// `showonline_apps` in aethercore.toml (both AetherDesk-managed copies). The
/// DLL re-reads it on every SpawnProcess — no Steam restart needed, and
/// nothing lands on the game's command line.
#[tauri::command]
pub fn set_aether_showonline(
    app: tauri::AppHandle,
    app_id: u32,
    enabled: bool,
) -> Result<String, String> {
    let steam_path = load_settings(&app).steam_path;
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

    let marker_active = aethercore_toml_paths(&app).iter().any(|p| {
        read_mode_apps(p, PresenceMode::ShowOnline)
            .map(|apps| apps.contains(&app_id))
            .unwrap_or(false)
    });
    if marker_active == enabled {
        return Ok(if enabled {
            format!("Aether showonline is already enabled for app {app_id}.")
        } else {
            format!("Aether showonline is already disabled for app {app_id}.")
        });
    }

    let choice = if enabled {
        Some(PresenceMode::ShowOnline)
    } else {
        None
    };
    let mut touched = false;
    for path in aethercore_toml_paths(&app) {
        if update_mode_in_toml(&path, app_id, choice)? {
            touched = true;
        }
    }

    crate::desk_log_info!(
        "steam",
        "Aether showonline {} for app {} (marker toml={})",
        if enabled { "enabled" } else { "disabled" },
        app_id,
        touched
    );
    Ok(if enabled {
        format!(
            "Aether showonline enabled for app {app_id} (friends will see what you're playing)."
        )
    } else {
        format!("Aether showonline disabled for app {app_id}.")
    })
}

use crate::core::presence_config::{
    aethercore_toml_paths, read_default_mode, read_mode_apps, set_default_mode_in_toml,
    update_mode_in_toml, PresenceMode,
};

/// True when the game is in exclude_apps (hard opt-out: the DLL fully
/// ignores the game, even with leftover argv tokens).
#[tauri::command]
pub fn get_aether_excluded(app: tauri::AppHandle, app_id: u32) -> Result<bool, String> {
    Ok(aethercore_toml_paths(&app).iter().any(|p| {
        read_mode_apps(p, PresenceMode::Excluded)
            .map(|apps| apps.contains(&app_id))
            .unwrap_or(false)
    }))
}

/// Adds/removes the app to/from the exclude_apps list (fully ignored by
/// Aether: no presence, no aetheronline).
#[tauri::command]
pub fn set_aether_excluded(
    app: tauri::AppHandle,
    app_id: u32,
    enabled: bool,
) -> Result<String, String> {
    let choice = if enabled {
        Some(PresenceMode::Excluded)
    } else {
        None
    };
    for path in aethercore_toml_paths(&app) {
        update_mode_in_toml(&path, app_id, choice)?;
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

/// True when the default policy is `default_mode = "showonline"`: every game
/// shows "now playing" to friends with no per-game configuration.
#[tauri::command]
pub fn get_presence_default_mode(app: tauri::AppHandle) -> Result<bool, String> {
    for path in aethercore_toml_paths(&app) {
        if let Some(v) = read_default_mode(&path) {
            return Ok(v);
        }
    }
    Ok(false)
}

/// Sets the default policy: "showonline" makes presence the
/// norm for every unlisted app; the arrays stay explicit overrides.
///
/// **IPC contract** (`docs/shared_contracts.md` §7): the parameter is named
/// `show_online` so the on-the-wire key is `showOnline` (Tauri 2 converts to
/// lowerCamelCase and accepts no snake_case fallback). The **value** written
/// in `aethercore.toml` stays the string `"showonline"`, which is the domain
/// token also read by AetherDLL (`Settings.cpp`): the two levels must neither
/// be confused nor "aligned".
#[tauri::command]
pub fn set_presence_default_mode(
    app: tauri::AppHandle,
    show_online: bool,
) -> Result<String, String> {
    for path in aethercore_toml_paths(&app) {
        set_default_mode_in_toml(&path, show_online)?;
    }
    crate::desk_log_info!(
        "steam",
        "presence default_mode = {}",
        if show_online { "showonline" } else { "none" }
    );
    Ok(if show_online {
        "Default mode is now showonline: every game (unless excluded/aetheronline) broadcasts what you're playing.".to_string()
    } else {
        "Default mode is now none: only apps explicitly listed get Aether presence.".to_string()
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
        if let Some(v) = crate::core::manifest_restore_config::read_manifest_restore_enabled(&path)
        {
            return Ok(v);
        }
    }
    Ok(true)
}

/// Enables/disables the startup refill of `Steam\depotcache` from the local
/// manifest backups. Applies on the next Steam start (the DLL runs the
/// restore once per Steam process, right after injection).
#[tauri::command]
pub fn set_manifest_restore_enabled(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<String, String> {
    for path in crate::core::manifest_restore_config::aethercore_toml_paths(&app) {
        crate::core::manifest_restore_config::set_manifest_restore_enabled_in_toml(&path, enabled)?;
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

    let selected = match picked {
        // Persist the canonical form immediately at the source: the frontend
        // saves this value verbatim, and normalization is idempotent.
        Some(file_path) => file_path_to_string(file_path)?.map(|path| normalize_steam_path(&path)),
        None => None,
    };
    if let Some(path) = &selected {
        crate::desk_log_info!("steam", "Steam folder picked: '{}'", path);
    }
    Ok(selected.filter(|path| !path.is_empty()))
}

fn pick_steam_folder_start_dir(app: &tauri::AppHandle) -> PathBuf {
    let configured = load_settings(app).steam_path;
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
