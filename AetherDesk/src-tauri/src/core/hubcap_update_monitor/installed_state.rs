//! Local installed-manifest evidence and pin realignment.

use crate::manifest::pins::{DepotManifestPin, LuaManifestPins};
use crate::steam::library::SteamLibraryScanner;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;
use tauri::AppHandle;

/// Newest non-empty manifest for one depot across Steam's two cache folders.
pub(crate) fn latest_installed_gid(steam_path: &str, depot_id: u32) -> Option<String> {
    let prefix = format!("{depot_id}_");
    let mut best: Option<(SystemTime, String)> = None;
    for directory in [
        PathBuf::from(steam_path).join("depotcache"),
        PathBuf::from(steam_path).join("config").join("depotcache"),
    ] {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.starts_with(&prefix) || !name.ends_with(".manifest") {
                continue;
            }
            let Some(gid) = name
                .strip_prefix(&prefix)
                .and_then(|stem| stem.strip_suffix(".manifest"))
                .and_then(|gid| gid.parse::<u64>().ok())
                .filter(|gid| *gid > 0)
            else {
                continue;
            };
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            if metadata.len() == 0 {
                continue;
            }
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            if best
                .as_ref()
                .map(|(time, _)| modified > *time)
                .unwrap_or(true)
            {
                best = Some((modified, gid.to_string()));
            }
        }
    }
    best.map(|(_, gid)| gid)
}

/// GID declared by Steam in `InstalledDepots` for one installed game.
pub(crate) fn acf_installed_gid(
    steam_path: &str,
    app_id: u32,
    depot_id: u32,
) -> Option<String> {
    SteamLibraryScanner::new(steam_path)
        .discover_library_paths()
        .into_iter()
        .find_map(|library| {
            crate::steam::acf::SteamAcfEditor::for_app(&library, app_id)
                .installed_depot_manifest(depot_id)
        })
}

/// Installed truth: ACF first, newest local cache entry only as fallback.
pub(crate) fn installed_gid_for_depot(
    steam_path: &str,
    app_id: u32,
    depot_id: u32,
) -> Option<String> {
    acf_installed_gid(steam_path, app_id, depot_id)
        .or_else(|| latest_installed_gid(steam_path, depot_id))
}

/// Realigns informational pins to Steam's installed truth and archives both
/// the pre-change and post-change Lua plus referenced manifests.
pub(crate) fn realign_pins_to_installed(
    steam_path: &str,
    app_id: u32,
    _mutation: &crate::core::game_mutations::MutationGuard,
) -> Result<usize, String> {
    let editor = LuaManifestPins::new(steam_path.to_string(), app_id);
    let content = editor.read_lua()?;
    let active_depots: Vec<u32> = LuaManifestPins::rows_for_manifest_sync(&content)
        .into_iter()
        .map(|row| row.app_id)
        .collect();
    let mut realignment = Vec::new();
    for row in LuaManifestPins::rows_from_content(&content) {
        if row.enabled || !active_depots.contains(&row.app_id) {
            continue;
        }
        if let Some(installed_gid) =
            installed_gid_for_depot(steam_path, app_id, row.app_id)
        {
            if installed_gid != row.manifest_id {
                realignment.push(DepotManifestPin {
                    depot_id: row.app_id,
                    manifest_id: installed_gid,
                });
            }
        }
    }
    if realignment.is_empty() {
        return Ok(0);
    }

    if let Ok(backup) = crate::core::backup::GameBackup::for_app(app_id) {
        let _ = backup.store_history_version(app_id, content.as_bytes());
    }
    let realigned = editor.realign_commented_pins(&realignment)?;
    if let (Ok(final_lua), Ok(backup)) = (
        editor.read_lua(),
        crate::core::backup::GameBackup::for_app(app_id),
    ) {
        let _ = backup.store_history_version(app_id, final_lua.as_bytes());
        let depotcache = PathBuf::from(steam_path).join("depotcache");
        let report = backup.backup_referenced_manifests(&final_lua, &depotcache);
        if report.copied > 0 || report.errors > 0 || report.missing > 0 {
            crate::desk_log_debug!(
                "hubcap-updates",
                "Post-realign manifest backup app_id={}: {} copied, {} unchanged, {} missing, {} errors",
                app_id,
                report.copied,
                report.unchanged,
                report.missing,
                report.errors
            );
        }
    }
    Ok(realigned)
}

pub(super) async fn sync_pins_after_steam_update(
    app: &AppHandle,
    steam_path: &str,
    app_id: u32,
) -> Result<usize, String> {
    let mutation = crate::core::game_mutations::acquire(
        std::path::Path::new(steam_path),
        app_id,
        "monitor-installed-pins",
    )
    .await?;
    crate::core::game_mutations::ensure_current_root(
        app,
        std::path::Path::new(steam_path),
    )?;
    let realigned = realign_pins_to_installed(steam_path, app_id, &mutation)?;
    if realigned == 0 {
        crate::desk_log_debug!(
            "hubcap-updates",
            "Pin sync found nothing to realign app_id={}",
            app_id
        );
        return Ok(0);
    }
    crate::desk_log_info!(
        "hubcap-updates",
        "Pin sync realigned app_id={} pins={}",
        app_id,
        realigned
    );
    crate::core::library_events::notify_lua_changed(
        app,
        crate::core::library_events::LibraryChangeOrigin::Versioning,
        [app_id],
    );
    Ok(realigned)
}
