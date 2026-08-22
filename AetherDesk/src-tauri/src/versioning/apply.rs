use std::path::PathBuf;

use crate::manifest::pins::{DepotManifestPin, LuaManifestPins};
use crate::steam::acf::SteamAcfEditor;
use crate::versioning::error::VersionError;
use crate::versioning::model::ApplyVersionReport;
use crate::versioning::queue::{self, PendingAcfEdit};

/// Progress callback used by the pipeline: `(percent, human message)`.
pub type ProgressFn<'a> = dyn Fn(u8, &str) + Send + Sync + 'a;

/// Applies the safely resolved portion of a build snapshot. Every step is
/// idempotent and the report always describes the real state that was reached:
///
///  1. validate the game has a stplug-in Lua
///  2. preserve the pre-change Lua into the AetherData backup tree
///     (`backup/<appid>/lua/history/`, deduplicated by content — no more
///     `.lua.bak` files in stplug-in)
///  3. pin every manifest resolved at or before the target into the Lua;
///     depots outside the available history remain byte-for-byte unchanged —
///     LumaCore picks the resolved changes up live
///  4. count which pinned `.manifest` files already exist locally
///  5. sync the ACF (`buildid` / `TargetBuildID` / `InstalledDepots[].manifest`);
///     when the ACF is missing or held by Steam the edit is queued and
///     retried in the background
///  6. verify the Lua pin count did not change and report
pub fn apply_build_version(
    app_id: u32,
    build_id: u64,
    steam_path: &str,
    library_path: &str,
    pins: &[DepotManifestPin],
    progress: &ProgressFn<'_>,
) -> Result<ApplyVersionReport, VersionError> {
    let lua = LuaManifestPins::new(steam_path.to_string(), app_id);
    if !lua.path_exists() {
        return Err(VersionError::LuaMissing(
            lua.lua_path().display().to_string(),
        ));
    }
    let before_rows = lua.rows_from_file().map_err(VersionError::Lua)?;
    let before_count = before_rows.len();
    crate::desk_log_info!(
        "versioning",
        "Apply pipeline for app {} (build {}): {} Lua depot row(s), {} pin(s) to write",
        app_id,
        build_id,
        before_count,
        pins.len()
    );

    progress(10, "Backing up the game Lua");
    // Niente più .lua.bak in stplug-in: il contenuto PRE-modifica viene
    // preservato nell'albero di backup AetherData (history/ se non già noto).
    let history_dir = crate::core::backup::GameBackup::for_app(app_id)
        .map(|backup| {
            let dir = backup.lua_dir().join(crate::core::backup::LUA_HISTORY_SUBDIR);
            if let Ok(pre_lua) = std::fs::read(lua.lua_path()) {
                if let Err(e) = backup.store_history_version(app_id, &pre_lua) {
                    crate::desk_log_warn!(
                        "versioning",
                        "Could not archive pre-change Lua for app {}: {}",
                        app_id,
                        e
                    );
                }
            }
            dir
        })
        .unwrap_or_else(|e| {
            crate::desk_log_warn!(
                "versioning",
                "Backup tree unavailable for app {}: {}",
                app_id,
                e
            );
            std::path::PathBuf::from("unavailable")
        });

    progress(30, "Pinning build manifests in the Lua");
    let apply_result = match lua.apply_build_pins(pins) {
        Ok(result) => {
            crate::desk_log_info!(
                "versioning",
                "Lua pins written for app {}: {} applied from the reconstructed build snapshot",
                app_id,
                result.applied_pins
            );
            result
        }
        Err(e) => {
            crate::desk_log_error!(
                "versioning",
                "Lua pin write failed for app {}: {}",
                app_id,
                e
            );
            return Err(VersionError::Lua(e));
        }
    };

    // La versione con i pin applicati è una VERSIONE MODIFICATA: va in
    // history/ (l'originale pristino nel backup non si tocca). Aggiornamento
    // immediato, non più al riavvio di Desk. Best-effort.
    if let Ok(final_lua) = std::fs::read_to_string(lua.lua_path()) {
        match crate::core::backup::GameBackup::for_app(app_id)
            .and_then(|backup| backup.store_history_version(app_id, final_lua.as_bytes()))
        {
            Ok(true) => crate::desk_log_info!(
                "versioning",
                "Lua version archived to history for app {} right after pin apply",
                app_id
            ),
            Ok(false) => crate::desk_log_info!(
                "versioning",
                "Lua version for app {} already known (history unchanged)",
                app_id
            ),
            Err(e) => crate::desk_log_warn!(
                "versioning",
                "History archive failed for app {} (will retry at next Desk start): {}",
                app_id,
                e
            ),
        }
    }

    progress(55, "Checking local manifest files");
    let (manifests_found, manifests_missing) = count_local_manifests(steam_path, pins);
    crate::desk_log_info!(
        "versioning",
        "Local manifests for app {}: {} of {} already in depotcache",
        app_id,
        manifests_found,
        pins.len()
    );

    progress(75, "Syncing the Steam ACF");
    let acf = SteamAcfEditor::for_app(library_path, app_id);
    let (acf_synced_now, acf_queued) = match acf.apply_build(build_id, pins) {
        Ok(()) => (true, false),
        Err(acf_error) => {
            crate::desk_log_warn!(
                "versioning",
                "ACF sync for app {} (build {}) deferred: {}",
                app_id,
                build_id,
                acf_error
            );
            queue::enqueue(PendingAcfEdit {
                app_id,
                build_id,
                pins: pins.to_vec(),
                steam_path: steam_path.to_string(),
                library_path: library_path.to_string(),
                queued_at: 0, // filled by enqueue
            })
            .map_err(|e| VersionError::Io {
                context: "acf queue",
                detail: e,
            })?;
            (false, true)
        }
    };

    progress(95, "Verifying");
    let after_count = lua.rows_from_file().map_err(VersionError::Lua)?.len();
    if after_count != before_count {
        crate::desk_log_error!(
            "versioning",
            "Verification failed for app {}: pin count {} -> {}",
            app_id,
            before_count,
            after_count
        );
        return Err(VersionError::Lua(format!(
            "Verification failed: setManifestid count changed from {} to {} after applying the build. Restore from {} to undo.",
            before_count,
            after_count,
            history_dir.display()
        )));
    }

    Ok(ApplyVersionReport {
        applied_pins: apply_result.applied_pins,
        manifests_found,
        manifests_missing,
        acf_synced_now,
        acf_queued,
        lua_backup_path: Some(history_dir.display().to_string()),
    })
}

/// Retries a queued ACF edit. `Ok(true)` = applied now; `Ok(false)` = still
/// not possible (missing/locked ACF); `Err` = give up on this pass.
pub fn try_apply_pending(edit: &PendingAcfEdit) -> Result<bool, VersionError> {
    let acf = SteamAcfEditor::for_app(&edit.library_path, edit.app_id);
    if !acf.exists() {
        return Ok(false);
    }
    acf.apply_build(edit.build_id, &edit.pins)
        .map_err(|e| VersionError::Io {
            context: "acf retry",
            detail: e,
        })
        .map(|_| true)
}

/// Counts how many pinned `.manifest` files are already present in Steam's
/// depotcache folders. Returns `(found, missing "depot:manifest" pairs)`.
fn count_local_manifests(steam_path: &str, pins: &[DepotManifestPin]) -> (usize, Vec<String>) {
    let mut search_dirs = vec![PathBuf::from(steam_path).join("depotcache")];
    search_dirs.push(PathBuf::from(steam_path).join("config").join("depotcache"));

    let mut found = 0usize;
    let mut missing: Vec<String> = Vec::new();
    for pin in pins {
        let file_name = format!("{}_{}.manifest", pin.depot_id, pin.manifest_id);
        let present = search_dirs.iter().any(|dir| dir.join(&file_name).exists());
        if present {
            found += 1;
        } else {
            missing.push(format!("{}:{}", pin.depot_id, pin.manifest_id));
        }
    }
    (found, missing)
}
