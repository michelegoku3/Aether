//! Removing a game's Lua from the Library.
//!
//! The Lua in `config\stplug-in` is AetherDLL's ownership/depot trigger; the
//! `appmanifest` is Steam's install record. A game can be fully installed and
//! still lose its Lua on request — that is how a legitimate owner who used the
//! Lua for DLC goes back to vanilla. The only install state that must refuse is
//! Steam actively writing the appmanifest, because the removal would race it.

use crate::core::backup::GameBackup;
use crate::steam::compat::SteamCompat;
use crate::steam::library::SteamLibraryScanner;
use std::fs;

/// What one removal actually did, for the command's message and log line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LuaRemovalReport {
    /// `<app_id>.lua` existed and was deleted.
    pub lua_removed: bool,
    /// A stale `<app_id>.lua.bak` was deleted alongside it.
    pub backup_removed: bool,
    /// The deleted Lua was archived into the game's backup history.
    pub archived: bool,
}

/// Deletes a game's Lua from `stplug-in`, archiving it first when possible.
///
/// Installed games are allowed on purpose: the content stays on disk and Steam
/// keeps its own record, while the Lua stops advertising the game's depots and
/// ownership. The Lua is archived through the same history the install paths
/// use, so a removal never destroys the only copy of a hand-installed file.
pub fn remove_lua_from_library(
    steam_path: &str,
    app_id: u32,
    backup: Option<&GameBackup>,
) -> Result<LuaRemovalReport, String> {
    let scanner = SteamLibraryScanner::new(steam_path.to_string());
    if scanner
        .install_projection()
        .get(&app_id)
        .is_some_and(|entry| entry.installing)
    {
        crate::desk_log_warn!(
            "library",
            "Refusing to remove the Lua for {}: Steam is still writing the appmanifest",
            crate::core::logger::format_appid(app_id)
        );
        return Err(format!(
            "Steam is still writing App ID {}. Wait for the download to finish, then remove its Lua.",
            app_id
        ));
    }

    let plugin_dir = SteamCompat::new(steam_path).get_plugin_dir();
    let lua_path = plugin_dir.join(format!("{app_id}.lua"));
    let backup_path = plugin_dir.join(format!("{app_id}.lua.bak"));

    let mut report = LuaRemovalReport::default();
    if lua_path.exists() {
        // Read before deleting: the archive is the only copy of a Lua that was
        // dropped into stplug-in by hand, outside every install pipeline.
        let bytes = match fs::read(&lua_path) {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                crate::desk_log_warn!(
                    "library",
                    "Could not read {} before removal: {}",
                    lua_path.display(),
                    error
                );
                None
            }
        };
        if let (Some(bytes), Some(backup)) = (bytes.as_deref(), backup) {
            match backup.store_history_version(app_id, bytes) {
                Ok(wrote) => {
                    report.archived = wrote;
                    crate::desk_log_info!(
                        "library",
                        "Archived the Lua of {} before removal (new history entry: {})",
                        crate::core::logger::format_appid(app_id),
                        wrote
                    );
                }
                Err(error) => crate::desk_log_warn!(
                    "library",
                    "Could not archive the Lua of {} before removal: {}",
                    crate::core::logger::format_appid(app_id),
                    error
                ),
            }
        }
        fs::remove_file(&lua_path).map_err(|error| {
            format!(
                "Failed to remove Lua file {}: {}",
                lua_path.display(),
                error
            )
        })?;
        report.lua_removed = true;
    }

    // A `.lua.bak` is a stale copy left by older tooling; the scanner ignores
    // it and it would keep advertising the removed game. Best-effort: the Lua
    // is already gone and a locked file must not fail the whole action.
    if backup_path.exists() {
        match fs::remove_file(&backup_path) {
            Ok(()) => report.backup_removed = true,
            Err(error) => crate::desk_log_warn!(
                "library",
                "Could not remove stale backup {}: {}",
                backup_path.display(),
                error
            ),
        }
    }

    crate::desk_log_info!(
        "library",
        "Lua removal for {}: lua_removed={} backup_removed={} archived={}",
        crate::core::logger::format_appid(app_id),
        report.lua_removed,
        report.backup_removed,
        report.archived
    );
    Ok(report)
}
