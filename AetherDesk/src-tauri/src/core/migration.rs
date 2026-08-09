// One-time migration da layout vecchi a quello unificato
// `%LOCALAPPDATA%\AetherDesk\AetherData` (fix v3 - 09/08/2026).
use crate::core::backup::GameBackup;
use crate::core::paths::LocalAppPaths;
use crate::manifest::pins::LuaManifestPins;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const LEGACY_LUA_BACKUPS_DIR: &str = "lua_backups";

#[derive(Debug, Default)]
pub struct MigrationReport {
    pub games: usize,
    pub lua_files: usize,
    pub manifest_files: usize,
}

// ---------------------------------------------------------------------------
// FIX v3: tutto unificato in <install_root>/AetherData
// Migra sia da Program Files legacy che da Roaming v2 verso la nuova location
// ---------------------------------------------------------------------------

/// Migra i dati dal fix v2 Roaming (`%APPDATA%\com.aether.desk`) verso
/// la cartella unificata `%LOCALAPPDATA%\AetherDesk\AetherData` (install_root).
/// Idempotente: copia solo file mancanti, non sovrascrive custom dell'utente.
pub fn migrate_roaming_to_local_install() -> Result<bool, String> {
    let roaming = LocalAppPaths::legacy_roaming_data_root();
    let local = LocalAppPaths::data_root();

    if roaming == local || !roaming.exists() || !roaming.is_dir() {
        return Ok(false);
    }
    if !local.exists() {
        fs::create_dir_all(&local)
            .map_err(|e| format!("Failed to create local data root {}: {}", local.display(), e))?;
    }
    let copied = copy_dir_recursive_missing_only(&roaming, &local)?;
    if copied > 0 {
        eprintln!(
            "[AetherDesk] migrated {} file(s) from Roaming {} to unified {}",
            copied,
            roaming.display(),
            local.display()
        );
    }
    if legacy_copied_successfully(&roaming, &local) {
        match fs::remove_dir_all(&roaming) {
            Ok(()) => eprintln!("[AetherDesk] removed Roaming legacy at {}", roaming.display()),
            Err(e) => eprintln!("[AetherDesk] keep Roaming (remove failed) {}: {}", roaming.display(), e),
        }
    }
    Ok(copied > 0)
}

/// Migra da vecchia install in Program Files verso la nuova Local install
/// e rimuove la vecchia AetherData se copiata con successo.
pub fn migrate_programfiles_to_local_install() -> Result<bool, String> {
    let candidates = [
        Path::new("C:\\Program Files\\AetherDesk\\AetherData").to_path_buf(),
        Path::new("C:\\Program Files (x86)\\AetherDesk\\AetherData").to_path_buf(),
    ];
    let local = LocalAppPaths::data_root();
    let mut did = false;
    for legacy in candidates {
        if legacy == local || !legacy.exists() || !legacy.is_dir() {
            continue;
        }
        if !local.exists() {
            fs::create_dir_all(&local)
                .map_err(|e| format!("Failed to create local data root {}: {}", local.display(), e))?;
        }
        let copied = copy_dir_recursive_missing_only(&legacy, &local)?;
        if copied > 0 {
            eprintln!(
                "[AetherDesk] migrated {} file(s) from Program Files {} to unified {}",
                copied,
                legacy.display(),
                local.display()
            );
            did = true;
        }
        if legacy_copied_successfully(&legacy, &local) {
            let _ = fs::remove_dir_all(&legacy);
            eprintln!("[AetherDesk] removed legacy Program Files AetherData at {}", legacy.display());
        }
    }
    Ok(did)
}

/// Rimuove le vecchie installazioni di Aether ovunque siano.
/// Chiamata ad ogni update/avvio: cerca installazioni legacy in Program Files
/// e altre location note e le elimina (l'app gira come admin, quindi può).
/// Non tocca mai la cartella di installazione corrente (`install_root`).
pub fn remove_legacy_install_folders() {
    let current = LocalAppPaths::install_root();
    let current_str = current.to_string_lossy().to_lowercase();

    let mut candidates: Vec<PathBuf> = vec![
        Path::new("C:\\Program Files\\AetherDesk").to_path_buf(),
        Path::new("C:\\Program Files (x86)\\AetherDesk").to_path_buf(),
        Path::new("C:\\Program Files\\Aether").to_path_buf(),
        Path::new("C:\\Program Files (x86)\\Aether").to_path_buf(),
    ];
    if let Some(local) = dirs::data_local_dir() {
        candidates.push(local.join("AetherDesk"));
        candidates.push(local.join("Aether"));
        candidates.push(local.join("Programs").join("AetherDesk"));
    }
    candidates.push(LocalAppPaths::legacy_roaming_data_root());

    for cand in candidates {
        let cand_str = cand.to_string_lossy().to_lowercase();
        if cand_str == current_str {
            continue;
        }
        if !cand.exists() {
            continue;
        }
        if current_str.starts_with(&cand_str) || cand_str.starts_with(&current_str) {
            if cand == current {
                continue;
            }
        }
        let is_legacy_install = cand.join("AetherDesk.exe").exists()
            || cand.join("Aether.exe").exists()
            || cand.join("AetherData").exists()
            || cand.join("Uninstall AetherDesk.exe").exists();
        let is_program_files_legacy = cand.to_string_lossy().contains("Program Files")
            && (cand.ends_with("AetherDesk") || cand.ends_with("Aether"));
        if !is_legacy_install && !is_program_files_legacy {
            continue;
        }

        // Evita di cancellare la nuova install se per qualche motivo coincide
        if cand == current {
            continue;
        }

        eprintln!("[AetherDesk] removing legacy installation at {}", cand.display());
        match fs::remove_dir_all(&cand) {
            Ok(()) => eprintln!("[AetherDesk] legacy installation removed: {}", cand.display()),
            Err(e) => eprintln!("[AetherDesk] failed to remove legacy {}: {}", cand.display(), e),
        }

        #[cfg(target_os = "windows")]
        {
            let _ = std::process::Command::new("reg")
                .args(["delete", "HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\AetherDesk", "/f"])
                .output();
            let _ = std::process::Command::new("reg")
                .args(["delete", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\AetherDesk", "/f"])
                .output();
            let _ = std::process::Command::new("reg")
                .args(["delete", "HKLM\\Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\AetherDesk", "/f"])
                .output();
        }
    }
}

fn copy_dir_recursive_missing_only(src: &Path, dst: &Path) -> Result<usize, String> {
    let mut count = 0usize;
    for entry in fs::read_dir(src).map_err(|e| format!("Failed to read {}: {}", src.display(), e))? {
        let entry = entry.map_err(|e| format!("Failed to read entry in {}: {}", src.display(), e))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            if !dst_path.exists() {
                fs::create_dir_all(&dst_path)
                    .map_err(|e| format!("Failed to create {}: {}", dst_path.display(), e))?;
            }
            count += copy_dir_recursive_missing_only(&src_path, &dst_path)?;
        } else if src_path.is_file() {
            if !dst_path.exists() {
                if let Err(e) = fs::copy(&src_path, &dst_path) {
                    eprintln!("[AetherDesk] failed to copy {} -> {}: {}", src_path.display(), dst_path.display(), e);
                } else {
                    count += 1;
                }
            }
        }
    }
    Ok(count)
}

fn legacy_copied_successfully(legacy: &Path, new: &Path) -> bool {
    fn check(src: &Path, dst: &Path) -> bool {
        let Ok(entries) = fs::read_dir(src) else { return true };
        for entry in entries.flatten() {
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if src_path.is_dir() {
                if !dst_path.is_dir() || !check(&src_path, &dst_path) {
                    return false;
                }
            } else if src_path.is_file() && !dst_path.exists() {
                return false;
            }
        }
        true
    }
    check(legacy, new)
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
    for entry in fs::read_dir(&legacy_dir)
        .map_err(|error| format!("Failed to read legacy folder {}: {}", legacy_dir.display(), error))?
    {
        let entry = entry
            .map_err(|error| format!("Failed to read legacy entry: {}", error))?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("lua")) {
            lua_files.push(path);
        }
    }

    for lua_path in &lua_files {
        let Some(app_id) = lua_path.file_stem().and_then(|stem| stem.to_string_lossy().parse::<u32>().ok())
        else {
            continue;
        };
        let content = fs::read_to_string(lua_path)
            .map_err(|error| format!("Failed to read Lua {}: {}", lua_path.display(), error))?;
        let rows = LuaManifestPins::rows_from_content(&content);
        let manifest_names: HashSet<String> = rows
            .iter()
            .map(|row| format!("{}_{}.manifest", row.app_id, row.manifest_id))
            .collect();
        let backup = GameBackup::for_app(app_id)?;
        backup.backup_lua_artifacts(app_id, &content, &[])?;
        report.lua_files += 1;
        let mut copied_manifests = 0usize;
        if depotcache_dir.is_dir() {
            for name in &manifest_names {
                let src = depotcache_dir.join(name);
                if src.is_file() {
                    let dest = backup.lua_dir().join(name);
                    fs::copy(&src, &dest).map_err(|error| {
                        format!("Failed to copy manifest {} to {}: {}", src.display(), dest.display(), error)
                    })?;
                    copied_manifests += 1;
                }
            }
        }
        report.manifest_files += copied_manifests;
        report.games += 1;
    }

    fs::remove_dir_all(&legacy_dir)
        .map_err(|error| format!("Failed to remove legacy folder {}: {}", legacy_dir.display(), error))?;

    Ok(report)
}

pub fn migrate_legacy_settings_if_needed(local_config_dir: &Path, legacy_config_dir: Option<&Path>) {
    let local_path = local_config_dir.join("settings.json");
    if local_path.exists() {
        return;
    }
    let Some(legacy_dir) = legacy_config_dir else {
        return;
    };
    let legacy_path = legacy_dir.join("settings.json");
    if !legacy_path.exists() {
        return;
    }
    if let Ok(content) = fs::read_to_string(&legacy_path) {
        if let Some(parent) = local_path.parent() {
            if fs::create_dir_all(parent).is_ok() && fs::write(&local_path, content).is_ok() {
                let _ = fs::remove_file(legacy_path);
            }
        }
    }
}

const OBSOLETE_COMPONENT_VERSION_DIR: &str = "component_versions";

pub fn remove_obsolete_component_version_dirs(app: &tauri::AppHandle) {
    let mut candidates = vec![
        LocalAppPaths::data_root().join(OBSOLETE_COMPONENT_VERSION_DIR),
        LocalAppPaths::legacy_roaming_data_root().join(OBSOLETE_COMPONENT_VERSION_DIR),
    ];
    if let Some(legacy_dir) = LocalAppPaths::legacy_app_data_dir(app) {
        candidates.push(legacy_dir.join(OBSOLETE_COMPONENT_VERSION_DIR));
    }
    for dir in candidates {
        if dir.is_dir() {
            match fs::remove_dir_all(&dir) {
                Ok(()) => eprintln!("[AetherDesk] removed obsolete component version folder {}", dir.display()),
                Err(error) => eprintln!(
                    "[AetherDesk] failed to remove obsolete folder {}: {}",
                    dir.display(),
                    error
                ),
            }
        }
    }
}

pub fn ensure_appearance_dirs() {
    if let Err(error) = crate::core::custom_css::ensure_default_assets() {
        eprintln!("[AetherDesk] failed to provision appearance folders: {error}");
    }
}

pub fn run_startup_migrations(app: &tauri::AppHandle) {
    if let Err(e) = migrate_roaming_to_local_install() {
        eprintln!("[AetherDesk] Roaming->Local migration failed: {e}");
    }
    if let Err(e) = migrate_programfiles_to_local_install() {
        eprintln!("[AetherDesk] ProgramFiles->Local migration failed: {e}");
    }
    // Nuovo: l'update rimuove le vecchie installazioni ovunque siano
    remove_legacy_install_folders();

    let config_dir = LocalAppPaths::config_dir();
    let legacy_config_dir = LocalAppPaths::legacy_app_config_dir(app);
    migrate_legacy_settings_if_needed(&config_dir, legacy_config_dir.as_deref());

    let roaming_config = LocalAppPaths::legacy_roaming_data_root().join("config");
    migrate_legacy_settings_if_needed(&config_dir, Some(&roaming_config));

    remove_obsolete_component_version_dirs(app);
    ensure_appearance_dirs();

    let steam_path = crate::core::settings::SettingsManager::new(app).load().steam_path;
    match migrate_legacy_lua_backups(std::path::Path::new(&steam_path)) {
        Ok(report) => {
            if report.games > 0 {
                eprintln!(
                    "[AetherDesk] migrated {} game(s) from lua_backups: {} lua, {} manifest",
                    report.games, report.lua_files, report.manifest_files
                );
            }
        }
        Err(error) => eprintln!("[AetherDesk] migration failed: {error}"),
    }
}
