// Startup migrations and self-healing that keep the on-disk contract valid.
//
// The critical pass runs work a command can depend on before the UI accepts
// IPC; the deferred pass runs the rest on a blocking worker once the window is
// interactive. Every step is idempotent and best-effort.

use crate::core::error::{codes, DeskError};
use std::fs;
use std::path::Path;

pub fn ensure_appearance_dirs() {
    if let Err(e) = crate::core::custom_css::ensure_default_assets() {
        eprintln!("[AetherDesk] appearance dirs failed: {e}");
    }
}

/// One-time policy migration: latest-version downloads no longer enable
/// Steam updates by default because the unauthenticated request-code path was
/// patched. The migration state is persisted in settings.json, alongside the
/// policy it describes, rather than in a standalone filesystem sentinel.
pub fn migrate_download_update_default_off(app: &tauri::AppHandle) {
    let manager = crate::core::settings::SettingsManager::new(app);
    let settings = manager.load();
    if settings.download_updates_default_off_migrated {
        return;
    }

    // Older settings files may have had the old default enabled. Move those
    // files to the safe default exactly once; an explicit choice can then be
    // enabled again through the normal Hubcap-key guard.
    let was_enabled = settings.download_games_with_updates_on;
    if let Err(error) = manager.update("migrate-update-default", |current| {
        if !current.download_updates_default_off_migrated {
            current.download_games_with_updates_on = false;
            current.download_updates_default_off_migrated = true;
        }
        Ok(())
    }) {
        crate::desk_log_warn!(
            "migration",
            "Could not persist download update policy migration: {}",
            error
        );
        return;
    }

    if was_enabled {
        crate::desk_log_info!(
            "migration",
            "Migrated download_games_with_updates_on to OFF; a valid Hubcap key is required to enable it again."
        );
    } else {
        crate::desk_log_info!(
            "migration",
            "Recorded download update policy migration in settings.json (default OFF)."
        );
    }
}

/// Migrations a command may depend on. These must finish before the UI
/// accepts IPC; everything else is deferred to `run_deferred_startup_tasks`.
pub fn run_critical_startup_migrations(app: &tauri::AppHandle) {
    crate::desk_log_info!(
        "migration",
        "Running critical AetherDesk startup migrations..."
    );
    migrate_download_update_default_off(app);
    ensure_appearance_dirs();
    // Adopt an auto-detected Steam installation when the stored value was
    // never configured (empty or stale legacy default). Runs before the
    // bridge below so the pointer targets the effective installation.
    adopt_detected_steam_path_if_unconfigured(app);
    ensure_aethercore_bridge(app);
}

/// Startup work that no command depends on. It runs on a blocking worker
/// after the window is interactive; each step is idempotent and isolated so
/// one failure cannot skip the rest.
pub fn run_deferred_startup_tasks(app: &tauri::AppHandle) {
    crate::desk_log_info!("migration", "Running deferred startup tasks...");
    run_isolated("shell shortcut icon sync", || {
        crate::core::custom_css::sync_shell_shortcuts(app)
    });
    run_isolated("Start Menu shortcut verification", || {
        ensure_start_menu_shortcut();
        Ok(())
    });
}

/// Runs one deferred step and turns any outcome into a log line. The
/// deferred pass is best-effort: a failing step must never abort later ones.
pub(crate) fn run_isolated<F>(label: &'static str, task: F)
where
    F: FnOnce() -> Result<(), String>,
{
    let started = std::time::Instant::now();
    match task() {
        Ok(()) => crate::desk_log_debug!(
            "migration",
            "Deferred startup step '{}' finished in {} ms",
            label,
            started.elapsed().as_millis()
        ),
        Err(error) => crate::desk_log_warn!(
            "migration",
            "Deferred startup step '{}' failed after {} ms: {}",
            label,
            started.elapsed().as_millis(),
            error
        ),
    }
}

/// Ensures that Windows Start Menu search finds this portable application by
/// creating or updating `AetherDesk.lnk` in `%APPDATA%\Microsoft\Windows\Start Menu\Programs`.
///
/// This function is idempotent and non-blocking: if the `.lnk` file already
/// exists and points to the running `AetherDesk.exe`, no disk modification is performed.
#[cfg(target_os = "windows")]
pub fn ensure_start_menu_shortcut() {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    crate::desk_log_info_once!(
        "migration",
        "Verifying Windows Start Menu shortcut AetherDesk.lnk"
    );

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let Ok(exe_path) = std::env::current_exe() else {
        return;
    };
    // Skip if running as temporary updater
    if exe_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case("aether_updater.exe"))
        .unwrap_or(false)
    {
        return;
    }

    let Some(work_dir) = exe_path.parent() else {
        return;
    };
    let Ok(appdata) = std::env::var("APPDATA") else {
        return;
    };

    let programs_dir = Path::new(&appdata)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs");

    if !programs_dir.exists() {
        if let Err(e) = fs::create_dir_all(&programs_dir) {
            eprintln!("[AetherDesk] Failed to create Start Menu Programs folder: {e}");
            return;
        }
    }

    let shortcut_path = programs_dir.join("AetherDesk.lnk");

    // PowerShell script to create or update the shortcut only when TargetPath differs.
    // Using Single Quote literals prevents PowerShell from interpreting backslashes.
    let script = format!(
        r#"$ws = New-Object -ComObject WScript.Shell; $s = $ws.CreateShortcut('{}'); if ($s.TargetPath -ne '{}') {{ $s.TargetPath = '{}'; $s.WorkingDirectory = '{}'; $s.Description = 'AetherDesk - Steam Library Manager'; $s.Save(); }}"#,
        shortcut_path.display().to_string().replace('\'', "''"),
        exe_path.display().to_string().replace('\'', "''"),
        exe_path.display().to_string().replace('\'', "''"),
        work_dir.display().to_string().replace('\'', "''"),
    );

    let _ = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

#[cfg(not(target_os = "windows"))]
pub fn ensure_start_menu_shortcut() {}

/// Startup self-heal for the Steam installation path (idempotent, best-effort).
///
/// * Stored path valid → log it as effective and do nothing.
/// * Stored path empty or a stale never-configured legacy default → adopt the
///   auto-detected installation (when one exists) and persist it.
/// * Stored path is an explicit custom value that is currently unreachable
///   (unplugged drive, …) → respect it untouched; the Settings UI surfaces the
///   problem instead of silently overriding the user's choice.
pub fn adopt_detected_steam_path_if_unconfigured(app: &tauri::AppHandle) {
    use crate::steam::resolve as steam_resolve;

    let manager = crate::core::settings::SettingsManager::new(app);
    let settings = manager.load();
    let current = steam_resolve::normalize_steam_path(&settings.steam_path);

    if let Ok(root) = steam_resolve::resolve_steam_path(&current) {
        crate::desk_log_info!(
            "migration",
            "Effective Steam path: {} (source=settings)",
            root.display()
        );
        return;
    }

    let never_configured = current.is_empty() || steam_resolve::is_legacy_default_path(&current);
    if !never_configured {
        crate::desk_log_warn!(
            "migration",
            "Configured Steam path '{}' is currently unreachable; keeping it untouched (check the drive or update Settings)",
            settings.steam_path
        );
        return;
    }

    match steam_resolve::detect_steam_installation() {
        Some((path, source)) => {
            crate::desk_log_info!(
                "migration",
                "Adopting auto-detected Steam path: {} (source={})",
                path.display(),
                source.as_log_label()
            );
            if let Err(error) = manager.update("detect-steam-path", |current_settings| {
                if current_settings.steam_path != settings.steam_path {
                    return Err(DeskError::coded(
                        codes::SETTINGS_CONFLICT,
                        "Steam path changed during detection",
                    ));
                }
                current_settings.steam_path = path.display().to_string();
                Ok(())
            }) {
                crate::desk_log_warn!(
                    "migration",
                    "Could not persist adopted Steam path: {}",
                    error
                );
            }
        }
        None => {
            crate::desk_log_warn!(
                "migration",
                "No Steam installation detected and no valid path configured; set it in Settings"
            );
        }
    }
}

/// Writes the locator pointer `<Steam>/aethercore/desk_path.cfg` (containing
/// `<AetherData>`'s path) so AetherDLL can discover configuration and logs,
/// migrating a legacy Steam-side `aethercore.toml` when needed.
///
/// Best-effort and idempotent. Validates the path first: an unconfigured or
/// invalid path logs a warning and writes nothing, instead of creating phantom
/// `aethercore` folders at wrong locations. Shared by startup migrations and the
/// settings-save flow (DRY).
pub fn ensure_steam_bridge_for_path(steam_path: &str) {
    let root = match crate::steam::resolve::resolve_steam_path(steam_path) {
        Ok(root) => root,
        Err(error) => {
            crate::desk_log_warn!(
                "migration",
                "Skipping AetherDLL bridge: {}",
                error.message(steam_path)
            );
            return;
        }
    };

    let data_root = crate::core::paths::LocalAppPaths::data_root();
    let config_dir = crate::core::paths::LocalAppPaths::config_dir();
    let toml_path = config_dir.join("aethercore.toml");
    if let Err(error) = fs::create_dir_all(&config_dir) {
        crate::desk_log_warn!("migration", "Could not create config dir: {}", error);
        return;
    }

    let steam_aethercore_dir = root.join("aethercore");
    if let Err(error) = fs::create_dir_all(&steam_aethercore_dir) {
        crate::desk_log_warn!(
            "migration",
            "Could not create {}: {}",
            steam_aethercore_dir.display(),
            error
        );
        return;
    }

    // Migrate legacy aethercore.toml from steam folder if needed
    let legacy_toml = steam_aethercore_dir.join("aethercore.toml");
    if let Err(error) = crate::core::config_document::initialize(
        &toml_path,
        Some(&legacy_toml),
        include_str!("../../assets/defaults/aethercore.toml"),
    ) {
        crate::desk_log_error!("migration", "DLL config initialization failed: {}", error);
    }

    // Write desk_path.cfg pointer
    let desk_cfg = steam_aethercore_dir.join("desk_path.cfg");
    if let Err(error) = fs::write(&desk_cfg, data_root.display().to_string()) {
        crate::desk_log_warn!(
            "migration",
            "Could not write {}: {}",
            desk_cfg.display(),
            error
        );
    }
}

/// Ensures that `AetherData/config/aethercore.toml` exists, the Steam-side
/// bridge pointer is in place (see [`ensure_steam_bridge_for_path`]), and both
/// TOML copies carry the canonical schema keys.
pub fn ensure_aethercore_bridge(app: &tauri::AppHandle) {
    let config_dir = crate::core::paths::LocalAppPaths::config_dir();
    let toml_path = config_dir.join("aethercore.toml");
    let _ = fs::create_dir_all(&config_dir);

    let steam_path = crate::core::settings::SettingsManager::new(app)
        .load()
        .steam_path;
    ensure_steam_bridge_for_path(&steam_path);

    if let Err(error) = crate::core::config_document::initialize(
        &toml_path,
        None,
        include_str!("../../assets/defaults/aethercore.toml"),
    ) {
        crate::desk_log_error!("migration", "DLL config initialization failed: {}", error);
    }

    // Schema evolution of the [presence] section: existing
    // installs predate showonline_apps/aetheronline_apps/exclude_apps and
    // default_mode. Insert ONLY the missing canonical keys (never overriding
    // user choices) so the DLL resolver and the Desk commands always find a
    // complete, predictable config. Idempotent, line-based, comment-safe.
    crate::core::presence_config::ensure_defaults(&toml_path);
    // Manifest restore on Steam startup ([manifest_cache] restore_on_startup,
    // default ON): insert only when missing, never overriding an explicit
    // user choice.
    crate::core::manifest_restore_config::ensure_defaults(&toml_path);
    if let Ok(root) = crate::steam::resolve::resolve_steam_path(&steam_path) {
        let legacy_toml = root.join("aethercore").join("aethercore.toml");
        if !legacy_toml.exists() {
            return;
        }
        crate::core::presence_config::ensure_defaults(&legacy_toml);
        crate::core::manifest_restore_config::ensure_defaults(&legacy_toml);
    }
}
