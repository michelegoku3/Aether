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
use std::path::{Path, PathBuf};

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

    /// Copia ricorsivamente solo file mancanti (non sovrascrive custom utente).
    /// Ritorna numero di file copiati.
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

    /// Verifica che ogni file in src esista in dst (usato per decidere se rimuovere src).
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

    /// Migra una directory src -> dst in modo idempotente.
    /// - Crea dst se manca
    /// - Copia solo file mancanti
    /// - Se tutto copiato, rimuove src (best-effort)
    /// Ritorna numero di file copiati.
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
// Helpers riusabili — legacy install detection
// ---------------------------------------------------------------------------
mod legacy_install {
    use super::*;

    pub fn candidates() -> Vec<PathBuf> {
        let mut v = vec![
            Path::new("C:\\Program Files\\AetherDesk").to_path_buf(),
            Path::new("C:\\Program Files (x86)\\AetherDesk").to_path_buf(),
            Path::new("C:\\Program Files\\Aether").to_path_buf(),
            Path::new("C:\\Program Files (x86)\\Aether").to_path_buf(),
        ];
        if let Some(local) = dirs::data_local_dir() {
            v.push(local.join("AetherDesk"));
            v.push(local.join("Aether"));
            v.push(local.join("Programs").join("AetherDesk"));
        }
        v.push(LocalAppPaths::legacy_roaming_data_root());
        v
    }

    pub fn is_legacy_install(path: &Path) -> bool {
        path.join("AetherDesk.exe").exists()
            || path.join("aether_desk.exe").exists()
            || path.join("Aether.exe").exists()
            || path.join("AetherData").exists()
            || path.join("Uninstall AetherDesk.exe").exists()
            || (path.to_string_lossy().contains("Program Files")
                && (path.ends_with("AetherDesk") || path.ends_with("Aether")))
    }

    /// Rimuove binari legacy con nome vecchio (aether_desk.exe) se esiste
    /// accanto al nuovo AetherDesk.exe nella stessa install. Chiamato a parte
    /// per non cancellare l'intera cartella corrente.
    pub fn cleanup_legacy_binary_in_current(current: &Path) {
        let legacy_bin = current.join("aether_desk.exe");
        let new_bin = current.join("AetherDesk.exe");
        if legacy_bin.exists() && new_bin.exists() {
            let _ = std::fs::remove_file(&legacy_bin);
            eprintln!("[AetherDesk] removed legacy binary {}", legacy_bin.display());
        }
        // Rimuovi anche vecchio uninstaller con nome lower-case se presente
        let legacy_uninst = current.join("uninstall.exe");
        // Tauri genera sempre "Uninstall AetherDesk.exe" con maiuscola, ma teniamo pulizia
        if legacy_uninst.exists() && current.join("Uninstall AetherDesk.exe").exists() {
            let _ = std::fs::remove_file(&legacy_uninst);
        }
    }
}

// ---------------------------------------------------------------------------
// Migrazioni specifiche — alta coesione, delegano a fs_utils
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

/// Rimuove vecchie installazioni ovunque (update). Non tocca mai install_root corrente.
pub fn remove_legacy_install_folders() {
    let current = LocalAppPaths::install_root();
    // Pulisci subito eventuale binario vecchio nella cartella corrente (aether_desk.exe -> AetherDesk.exe)
    legacy_install::cleanup_legacy_binary_in_current(&current);
    let cur = current.to_string_lossy().to_lowercase();
    for cand in legacy_install::candidates() {
        if cand == current || !cand.exists() {
            continue;
        }
        let s = cand.to_string_lossy().to_lowercase();
        if cur.starts_with(&s) || s.starts_with(&cur) {
            continue;
        }
        if !legacy_install::is_legacy_install(&cand) {
            continue;
        }
        eprintln!("[AetherDesk] removing legacy installation {}", cand.display());
        if let Err(e) = fs::remove_dir_all(&cand) {
            eprintln!("[AetherDesk] failed to remove {}: {}", cand.display(), e);
        } else {
            eprintln!("[AetherDesk] legacy removed {}", cand.display());
        }
        #[cfg(target_os = "windows")]
        cleanup_uninstall_registry();
    }
}

#[cfg(target_os = "windows")]
fn cleanup_uninstall_registry() {
    for key in [
        "HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\AetherDesk",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\AetherDesk",
        "HKLM\\Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\AetherDesk",
    ] {
        let _ = std::process::Command::new("reg").args(["delete", key, "/f"]).output();
    }
}
#[cfg(not(target_os = "windows"))]
fn cleanup_uninstall_registry() {}

// ---------------------------------------------------------------------------
// Altre migrazioni esistenti (invariate nella logica, solo formattazione)
// ---------------------------------------------------------------------------

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

pub fn reset_antivirus_exclusion_flag(app: &tauri::AppHandle) {
    let m = crate::core::settings::SettingsManager::new(app);
    let mut s = m.load();
    if s.antivirus_exclusion_done {
        s.antivirus_exclusion_done = false;
        if let Err(e) = m.save(&s) {
            eprintln!("[AetherDesk] reset antivirus flag failed: {e}");
        } else {
            eprintln!("[AetherDesk] reset antivirus_exclusion_done to false");
        }
    }
}

pub fn run_startup_migrations(app: &tauri::AppHandle) {
    if let Err(e) = migrate_roaming_to_local_install() { eprintln!("[AetherDesk] Roaming->Local failed: {e}"); }
    if let Err(e) = migrate_programfiles_to_local_install() { eprintln!("[AetherDesk] PF->Local failed: {e}"); }
    remove_legacy_install_folders();
    reset_antivirus_exclusion_flag(app);
    let config_dir = LocalAppPaths::config_dir();
    let legacy_config_dir = LocalAppPaths::legacy_app_config_dir(app);
    migrate_legacy_settings_if_needed(&config_dir, legacy_config_dir.as_deref());
    let roaming_config = LocalAppPaths::legacy_roaming_data_root().join("config");
    migrate_legacy_settings_if_needed(&config_dir, Some(&roaming_config));
    remove_obsolete_component_version_dirs(app);
    ensure_appearance_dirs();
    let steam_path = crate::core::settings::SettingsManager::new(app).load().steam_path;
    match migrate_legacy_lua_backups(Path::new(&steam_path)) {
        Ok(r) if r.games > 0 => eprintln!("[AetherDesk] migrated {} lua games", r.games),
        Err(e) => eprintln!("[AetherDesk] lua migration failed: {e}"),
        _ => {}
    }
}
