// One-time migration from older AetherDesk data layouts to the centralized
// `AetherData/backup/<app_id>/` structure.
//
// Older builds kept downloaded Lua files in `AetherData/lua_backups/<app_id>.lua`
// and the matching Steam `.manifest` files lived in `<steam>/depotcache/`.
// This module moves both into the new per-game backup tree and then removes
// the legacy `lua_backups` folder.
//
// It is intentionally a small, pure filesystem service: it takes the Steam
// path as input, uses `LocalAppPaths`/`GameBackup` for destinations, and is
// safe to call at startup (it is a no-op when there is nothing to migrate).
use crate::core::backup::GameBackup;
use crate::core::paths::LocalAppPaths;
use crate::manifest::pins::LuaManifestPins;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const LEGACY_LUA_BACKUPS_DIR: &str = "lua_backups";

/// Summary of what the migration did (0s when there was nothing to migrate).
#[derive(Debug, Default)]
pub struct MigrationReport {
    pub games: usize,
    pub lua_files: usize,
    pub manifest_files: usize,
}

/// Migrate `AetherData/lua_backups/*.lua` (and their Steam depotcache manifests)
/// into the centralized `backup` tree, then delete the legacy folder.
///
/// Returns early (empty report) if the legacy folder does not exist, so calling
/// this on every startup is cheap and idempotent.
pub fn migrate_legacy_lua_backups(steam_path: &Path) -> Result<MigrationReport, String> {
    let legacy_dir = LocalAppPaths::data_root().join(LEGACY_LUA_BACKUPS_DIR);
    if !legacy_dir.is_dir() {
        return Ok(MigrationReport::default());
    }

    let depotcache_dir = steam_path.join("depotcache");
    let mut report = MigrationReport::default();

    // Collect every `<app_id>.lua` present in the legacy folder first so the
    // folder can be safely removed only after all files have been migrated.
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
            continue; // legacy file whose name is not "<app_id>.lua" — skip
        };

        let content = fs::read_to_string(lua_path)
            .map_err(|error| format!("Failed to read Lua {}: {}", lua_path.display(), error))?;

        // Reuse the existing Lua manifest parser to discover the depot manifest
        // files that belong to this game (they live in Steam/depotcache).
        let rows = LuaManifestPins::rows_from_content(&content);
        let manifest_names: HashSet<String> = rows
            .iter()
            .map(|row| format!("{}_{}.manifest", row.app_id, row.manifest_id))
            .collect();

        let backup = GameBackup::for_app(app_id)?;

        // Copy the Lua into the centralized lua folder (atomic write).
        backup.backup_lua_artifacts(app_id, &content, &[])?;
        report.lua_files += 1;

        // Copy any matching manifest files present in depotcache.
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

    // Only now that everything has been copied, remove the legacy folder.
    fs::remove_dir_all(&legacy_dir)
        .map_err(|error| format!("Failed to remove legacy folder {}: {}", legacy_dir.display(), error))?;

    Ok(report)
}

// ---------------------------------------------------------------------------
// Cache freshness policy (documentation-only section)
// ---------------------------------------------------------------------------
//
// Cache invalidation is self-describing and lives WITH the cache files
// themselves: `store_search_cache.json` and `denuvo_cache.json` each carry
// the writing app's version (read from `tauri.conf.json` at runtime via
// `AppHandle::package_info().version`). A build change resets the files on
// load — no sidecar stamp file, nothing to migrate here.
// See `store::cache` and `store::drm` for the enforcement points.

// ---------------------------------------------------------------------------
// Legacy settings migration
// ---------------------------------------------------------------------------

/// Move a legacy `settings.json` (Tauri default app_config dir) into the
/// centralized `AetherData/config/` folder. Idempotent no-op when the local
/// file already exists or there is no legacy file. This logic used to live
/// inside `SettingsManager`; it is centralized here so every migration helper
/// lives in one module.
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

// ---------------------------------------------------------------------------
// Obsolete component version directory
// ---------------------------------------------------------------------------

const OBSOLETE_COMPONENT_VERSION_DIR: &str = "component_versions";

/// Delete the obsolete `component_versions/` folder (both the centralized
/// AetherData location and the legacy app-data location). It used to hold the
/// AetherDLL version bookmark (`aetherdll_version.txt`) — retired because the
/// version now lives INSIDE the .dll files themselves (PE version resource), so
/// nothing writes there anymore. Idempotent no-op when there is nothing to
/// remove; a deletion failure degrades to a log line, never blocking startup.
pub fn remove_obsolete_component_version_dirs(app: &tauri::AppHandle) {
    let mut candidates = vec![LocalAppPaths::data_root().join(OBSOLETE_COMPONENT_VERSION_DIR)];
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

// ---------------------------------------------------------------------------
// Startup hub
// ---------------------------------------------------------------------------

/// Ensure the appearance asset folders (`config/themes/`, `config/wallpapers/`)
/// exist and are seeded with the embedded Cyberpunk defaults on first run.
/// Idempotent and failure-tolerant (logs, never blocks startup).
pub fn ensure_appearance_dirs() {
    if let Err(error) = crate::core::custom_css::ensure_default_assets() {
        eprintln!("[AetherDesk] failed to provision appearance folders: {error}");
    }
}

/// Run every startup migration in one place (settings → data layout → obsolete
/// leftovers). Each step is idempotent and degrades to a log line on failure, so
/// a broken migration never prevents the app from starting.
///
/// Note: cache freshness is NOT here — it is self-describing inside each
/// cache file (the writing app's version), enforced at read time by the owning
/// caches; see the "Cache freshness policy" comment above.
pub fn run_startup_migrations(app: &tauri::AppHandle) {
    let config_dir = LocalAppPaths::config_dir();
    let legacy_config_dir = LocalAppPaths::legacy_app_config_dir(app);
    migrate_legacy_settings_if_needed(&config_dir, legacy_config_dir.as_deref());

    remove_obsolete_component_version_dirs(app);

    // Create + seed config/themes and config/wallpapers on every start.
    ensure_appearance_dirs();

    // Load AFTER the settings migration so the steam path is the migrated one.
    let steam_path = crate::core::settings::SettingsManager::new(app)
        .load()
        .steam_path;

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
