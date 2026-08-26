// One-time migration da layout vecchi a quello unificato
// `%LOCALAPPDATA%\AetherDesk\AetherData` (fix v3 - 09/08/2026).
//
// Principi: DRY, alta coesione, basso accoppiamento, singola responsabilità.
// Ogni migrazione è idempotente, best-effort e logga senza bloccare l'avvio.

use crate::core::backup::GameBackup;
use crate::core::paths::LocalAppPaths;
use crate::manifest::pins::LuaManifestPins;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const LEGACY_LUA_BACKUPS_DIR: &str = "lua_backups";
const OBSOLETE_COMPONENT_VERSION_DIR: &str = "component_versions";

#[derive(Debug, Default)]
pub struct MigrationReport {
    pub games: usize,
    pub lua_files: usize,
    pub manifest_files: usize,
}

// ---------------------------------------------------------------------------
// Helpers riusabili (DRY) — filesystem
// ---------------------------------------------------------------------------
mod fs_utils {
    use super::*;

    pub fn copy_missing_only(src: &Path, dst: &Path) -> Result<usize, String> {
        let mut count = 0;
        for entry in fs::read_dir(src).map_err(|e| format!("read {}: {}", src.display(), e))? {
            let entry = entry.map_err(|e| format!("entry in {}: {}", src.display(), e))?;
            let s = entry.path();
            let d = dst.join(entry.file_name());
            if s.is_dir() {
                if !d.exists() {
                    fs::create_dir_all(&d).map_err(|e| format!("mkdir {}: {}", d.display(), e))?;
                }
                count += copy_missing_only(&s, &d)?;
            } else if s.is_file() && !d.exists() {
                match fs::copy(&s, &d) {
                    Ok(_) => count += 1,
                    Err(e) => eprintln!("[AetherDesk] copy failed {} -> {}: {}", s.display(), d.display(), e),
                }
            }
        }
        Ok(count)
    }

    pub fn is_fully_copied(src: &Path, dst: &Path) -> bool {
        let Ok(entries) = fs::read_dir(src) else { return true };
        for e in entries.flatten() {
            let s = e.path();
            let d = dst.join(e.file_name());
            if s.is_dir() {
                if !d.is_dir() || !is_fully_copied(&s, &d) {
                    return false;
                }
            } else if s.is_file() && !d.exists() {
                return false;
            }
        }
        true
    }

    pub fn migrate_dir(src: &Path, dst: &Path, label: &str) -> Result<usize, String> {
        if src == dst || !src.is_dir() {
            return Ok(0);
        }
        if !dst.exists() {
            fs::create_dir_all(dst).map_err(|e| format!("mkdir {}: {}", dst.display(), e))?;
        }
        let n = copy_missing_only(src, dst)?;
        if n > 0 {
            eprintln!("[AetherDesk] migrated {n} file(s) {label} {} -> {}", src.display(), dst.display());
        }
        if is_fully_copied(src, dst) {
            match fs::remove_dir_all(src) {
                Ok(()) => eprintln!("[AetherDesk] removed legacy {label} {}", src.display()),
                Err(e) => eprintln!("[AetherDesk] keep legacy {label} {}: {}", src.display(), e),
            }
        }
        Ok(n)
    }
}

// ---------------------------------------------------------------------------
// Helpers — legacy file cleanup
// ---------------------------------------------------------------------------
mod legacy_install {
    use super::*;

    /// Removes leftover binaries from a previous installer-based install that
    /// have no meaning in the portable distribution (old exe name, NSIS
    /// uninstaller). Best-effort and idempotent.
    pub fn cleanup_legacy_binary_in_current(current: &Path) {
        let legacy_bin = current.join("aether_desk.exe");
        let new_bin = current.join("AetherDesk.exe");
        if legacy_bin.exists() && new_bin.exists() {
            let _ = std::fs::remove_file(&legacy_bin);
            eprintln!("[AetherDesk] removed legacy binary {}", legacy_bin.display());
        }
        let legacy_uninst = current.join("uninstall.exe");
        if legacy_uninst.exists() && current.join("Uninstall AetherDesk.exe").exists() {
            let _ = std::fs::remove_file(&legacy_uninst);
        }
    }
}

// ---------------------------------------------------------------------------
// Migrazioni specifiche
// ---------------------------------------------------------------------------

pub fn migrate_roaming_to_local_install() -> Result<bool, String> {
    let n = fs_utils::migrate_dir(
        &LocalAppPaths::legacy_roaming_data_root(),
        &LocalAppPaths::data_root(),
        "Roaming",
    )?;
    Ok(n > 0)
}

pub fn migrate_programfiles_to_local_install() -> Result<bool, String> {
    let mut total = 0;
    for src in [
        Path::new("C:\\Program Files\\AetherDesk\\AetherData").to_path_buf(),
        Path::new("C:\\Program Files (x86)\\AetherDesk\\AetherData").to_path_buf(),
    ] {
        total += fs_utils::migrate_dir(&src, &LocalAppPaths::data_root(), "ProgramFiles")?;
    }
    Ok(total > 0)
}

/// Removes leftover binaries from a previous installer-based install inside the
/// current (portable) folder. In portable mode there is no system install to
/// clean and no uninstall registry to touch, so this only tidies the folder.
pub fn remove_legacy_install_folders() {
    let current = LocalAppPaths::install_root();
    legacy_install::cleanup_legacy_binary_in_current(&current);
}

pub fn migrate_legacy_lua_backups(steam_path: &Path) -> Result<MigrationReport, String> {
    let candidates = [
        LocalAppPaths::data_root().join(LEGACY_LUA_BACKUPS_DIR),
        LocalAppPaths::legacy_roaming_data_root().join(LEGACY_LUA_BACKUPS_DIR),
    ];
    let legacy_dir = candidates.iter().find(|p| p.is_dir()).cloned().unwrap_or(candidates[0].clone());
    if !legacy_dir.is_dir() {
        return Ok(MigrationReport::default());
    }
    let depotcache_dir = steam_path.join("depotcache");
    let mut report = MigrationReport::default();
    let mut lua_files = Vec::new();
    for entry in fs::read_dir(&legacy_dir).map_err(|e| format!("read {}: {}", legacy_dir.display(), e))? {
        let e = entry.map_err(|e| format!("entry: {}", e))?;
        let p = e.path();
        if p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("lua")) {
            lua_files.push(p);
        }
    }
    for lua_path in &lua_files {
        let Some(app_id) = lua_path.file_stem().and_then(|s| s.to_string_lossy().parse::<u32>().ok()) else { continue };
        let content = fs::read_to_string(lua_path).map_err(|e| format!("read {}: {}", lua_path.display(), e))?;
        let rows = LuaManifestPins::rows_from_content(&content);
        let names: HashSet<String> = rows.iter().map(|r| format!("{}_{}.manifest", r.app_id, r.manifest_id)).collect();
        let backup = GameBackup::for_app(app_id)?;
        backup.backup_lua_artifacts(app_id, &content, &[])?;
        report.lua_files += 1;
        let mut c = 0;
        if depotcache_dir.is_dir() {
            for name in &names {
                let src = depotcache_dir.join(name);
                if src.is_file() {
                    let dst = backup.lua_dir().join(name);
                    fs::copy(&src, &dst).map_err(|e| format!("copy {}: {}", src.display(), e))?;
                    c += 1;
                }
            }
        }
        report.manifest_files += c;
        report.games += 1;
    }
    fs::remove_dir_all(&legacy_dir).map_err(|e| format!("rm {}: {}", legacy_dir.display(), e))?;
    Ok(report)
}

pub fn migrate_legacy_settings_if_needed(local_config_dir: &Path, legacy_config_dir: Option<&Path>) {
    let local_path = local_config_dir.join("settings.json");
    if local_path.exists() { return; }
    let Some(legacy_dir) = legacy_config_dir else { return; };
    let legacy_path = legacy_dir.join("settings.json");
    if !legacy_path.exists() { return; }
    if let Ok(content) = fs::read_to_string(&legacy_path) {
        if let Some(parent) = local_path.parent() {
            if fs::create_dir_all(parent).is_ok() && fs::write(&local_path, content).is_ok() {
                let _ = fs::remove_file(legacy_path);
            }
        }
    }
}

pub fn remove_obsolete_component_version_dirs(app: &tauri::AppHandle) {
    let mut candidates = vec![
        LocalAppPaths::data_root().join(OBSOLETE_COMPONENT_VERSION_DIR),
        LocalAppPaths::legacy_roaming_data_root().join(OBSOLETE_COMPONENT_VERSION_DIR),
    ];
    if let Some(d) = LocalAppPaths::legacy_app_data_dir(app) {
        candidates.push(d.join(OBSOLETE_COMPONENT_VERSION_DIR));
    }
    for dir in candidates {
        if dir.is_dir() {
            match fs::remove_dir_all(&dir) {
                Ok(()) => eprintln!("[AetherDesk] removed obsolete {}", dir.display()),
                Err(e) => eprintln!("[AetherDesk] failed rm {}: {}", dir.display(), e),
            }
        }
    }
}

pub fn ensure_appearance_dirs() {
    if let Err(e) = crate::core::custom_css::ensure_default_assets() {
        eprintln!("[AetherDesk] appearance dirs failed: {e}");
    }
}

// FIX antivirus: torna al comportamento stock (fresh install = false).
// Non forza più il reset ad ogni avvio — il bug del popup ad ogni "Apply crack"
// era dovuto al reset incondizionato. Ora il flag resta come da settings.json.
pub fn reset_antivirus_exclusion_flag(_app: &tauri::AppHandle) {
    // Intentionally no-op: fresh install ha default false (mostra popup una volta),
    // update mantiene il valore precedente. Rimuove il bug del popup ricorrente.
}

pub fn run_startup_migrations(app: &tauri::AppHandle) {
    crate::desk_log_info!("migration", "Running AetherDesk startup migrations...");
    if let Err(e) = migrate_roaming_to_local_install() { eprintln!("[AetherDesk] Roaming->Local failed: {e}"); }
    if let Err(e) = migrate_programfiles_to_local_install() { eprintln!("[AetherDesk] PF->Local failed: {e}"); }
    remove_legacy_install_folders();
    reset_antivirus_exclusion_flag(app);
    // Pulizia una-tantum: la release desk-1.0.1 (commit d4ccbb1) scriveva un
    // sentinel `.v3_antivirus_reset_done` in AetherData per forzare il reset del
    // flag antivirus. Ora reset_antivirus_exclusion_flag è un no-op, ma il file
    // resta su disco per chi aveva già installato quella build: lo rimuoviamo.
    let _ = fs::remove_file(LocalAppPaths::data_root().join(".v3_antivirus_reset_done"));
    let config_dir = LocalAppPaths::config_dir();
    let legacy_config_dir = LocalAppPaths::legacy_app_config_dir(app);
    migrate_legacy_settings_if_needed(&config_dir, legacy_config_dir.as_deref());
    let roaming_config = LocalAppPaths::legacy_roaming_data_root().join("config");
    migrate_legacy_settings_if_needed(&config_dir, Some(&roaming_config));
    remove_obsolete_component_version_dirs(app);
    ensure_appearance_dirs();
    ensure_aethercore_bridge(app);
    let steam_path = crate::core::settings::SettingsManager::new(app).load().steam_path;
    match migrate_legacy_lua_backups(Path::new(&steam_path)) {
        Ok(r) if r.games > 0 => eprintln!("[AetherDesk] migrated {} lua games", r.games),
        Err(e) => eprintln!("[AetherDesk] lua migration failed: {e}"),
        _ => {}
    }
    ensure_start_menu_shortcut();
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

    crate::desk_log_info_once!("migration", "Verifying Windows Start Menu shortcut AetherDesk.lnk");

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let Ok(exe_path) = std::env::current_exe() else { return; };
    // Skip if running as temporary updater
    if exe_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case("aether_updater.exe"))
        .unwrap_or(false)
    {
        return;
    }

    let Some(work_dir) = exe_path.parent() else { return; };
    let Ok(appdata) = std::env::var("APPDATA") else { return; };

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
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

#[cfg(not(target_os = "windows"))]
pub fn ensure_start_menu_shortcut() {}

/// Ensures that `AetherData/config/aethercore.toml` exists and writes a locator
/// pointer file `<Steam>/aethercore/desk_path.cfg` containing `<AetherData>`'s path
/// so AetherDLL can discover configuration and logs cleanly.
pub fn ensure_aethercore_bridge(app: &tauri::AppHandle) {
    let data_root = crate::core::paths::LocalAppPaths::data_root();
    let config_dir = crate::core::paths::LocalAppPaths::config_dir();
    let toml_path = config_dir.join("aethercore.toml");
    let _ = fs::create_dir_all(&config_dir);

    let steam_path = crate::core::settings::SettingsManager::new(app).load().steam_path;
    if !steam_path.trim().is_empty() {
        let steam_aethercore_dir = Path::new(&steam_path).join("aethercore");
        let _ = fs::create_dir_all(&steam_aethercore_dir);

        // Migrate legacy aethercore.toml from steam folder if needed
        let legacy_toml = steam_aethercore_dir.join("aethercore.toml");
        if legacy_toml.exists() && !toml_path.exists() {
            let _ = fs::copy(&legacy_toml, &toml_path);
            let _ = fs::remove_file(&legacy_toml);
        }

        // Write desk_path.cfg pointer
        let desk_cfg = steam_aethercore_dir.join("desk_path.cfg");
        let _ = fs::write(&desk_cfg, data_root.display().to_string());
    }

    if !toml_path.exists() {
        const DEFAULT_AETHERCORE_TOML: &str =
            include_str!("../../assets/defaults/aethercore.toml");
        let _ = fs::write(&toml_path, DEFAULT_AETHERCORE_TOML);
    }

    // Schema evolution of the [presence] section (docs/05 §11-§13): existing
    // installs predate showonline_apps/aetheronline_apps/exclude_apps and
    // default_mode. Insert ONLY the missing canonical keys (never overriding
    // user choices) so the DLL resolver and the Desk commands always find a
    // complete, predictable config. Idempotent, line-based, comment-safe.
    // Rename legacy [presence] keys (onlinefix_apps -> aetheronline_apps,
    // onlinefix_persona_patch -> aetheronline_persona_patch) before inserting
    // defaults, so pre-rename installs migrate to the aetheronline naming.
    crate::core::presence_config::migrate_legacy_presence_keys(&toml_path);
    crate::core::presence_config::ensure_defaults(&toml_path);
    if !steam_path.trim().is_empty() {
        let legacy_toml = Path::new(&steam_path)
            .join("aethercore")
            .join("aethercore.toml");
        crate::core::presence_config::migrate_legacy_presence_keys(&legacy_toml);
        crate::core::presence_config::ensure_defaults(&legacy_toml);
    }
}
