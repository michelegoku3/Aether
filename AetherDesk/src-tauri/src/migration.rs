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
use crate::backup::GameBackup;
use crate::local_app_paths::LocalAppPaths;
use crate::lua_manifest_pins::LuaManifestPins;
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
