use serde::Serialize;
use std::path::PathBuf;
use std::time::Instant;

use crate::core::settings::SettingsManager;
use crate::manifest::pins::{pins_from_rows, DepotManifestPin, LuaManifestPins};
use crate::providers::hubcap::HubcapClient;
use crate::steam::compat::SteamCompat;

/// Result of an explicit, app-scoped manifest repair. Automatic package
/// updates use the ACF-driven coordinator instead of calling this for every
/// game on a timer.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameManifestSyncReport {
    pub app_id: u32,
    pub pins: usize,
    pub missing: usize,
    pub generated: usize,
    pub installed: usize,
    pub skipped: bool,
}

/// Local-first repair for one managed game. It never discovers the whole
/// library, never uses the obsolete request-code path, and asks Hubcap only
/// for exact manifest files that are missing from Steam's depotcache.
#[tauri::command]
pub async fn sync_hubcap_game_manifest(
    app: tauri::AppHandle,
    app_id: u32,
) -> Result<GameManifestSyncReport, String> {
    if app_id == 0 {
        return Err("A valid AppID is required".to_string());
    }

    let settings = SettingsManager::new(&app).load();
    if settings.steam_path.trim().is_empty() {
        return Err("Manifest repair cannot run because the Steam path is empty".to_string());
    }

    let started = Instant::now();
    let lua = LuaManifestPins::new(settings.steam_path.clone(), app_id);
    let content = std::fs::read_to_string(lua.lua_path())
        .map_err(|error| format!("Could not read Lua for app {app_id}: {error}"))?;
    LuaManifestPins::validate_content(&content)?;
    let pins: Vec<DepotManifestPin> =
        pins_from_rows(LuaManifestPins::rows_for_manifest_sync(&content));

    let mut report = GameManifestSyncReport {
        app_id,
        pins: pins.len(),
        ..Default::default()
    };
    if pins.is_empty() {
        report.skipped = true;
        return Ok(report);
    }

    // Shared local-first resolution: backup/secondary-cache hits are restored
    // into depotcache first; only genuinely absent manifests are generated
    // through the configured Hubcap key.
    let generation = if settings.hubcap_api_key.trim().is_empty() {
        crate::manifest::resolver::Generation::LocalOnly
    } else {
        crate::manifest::resolver::Generation::SettingsKey(settings.hubcap_api_key.clone())
    };
    let resolution = crate::manifest::resolver::resolve(
        crate::manifest::resolver::ManifestRequest {
            steam_path: settings.steam_path.clone(),
            app_id,
            pins: pins.clone(),
            generation,
        },
    )
    .await?;
    report.missing = resolution.generated.len() + resolution.missing.len();
    if !resolution.is_complete() {
        return Err("A valid authenticated Hubcap API key is required for missing manifests".to_string());
    }
    if resolution.generated.is_empty() {
        crate::desk_log_debug!(
            "hubcap",
            "Explicit game manifest repair already local app_id={} pins={} elapsed_ms={}",
            app_id,
            report.pins,
            started.elapsed().as_millis()
        );
        return Ok(report);
    }

    report.generated = resolution.generated.len();
    let expected = resolution.generated.len();
    let generated = resolution.generated;
    report.installed = SteamCompat::new(settings.steam_path.clone())
        .install_manifest_files(&generated)?;
    if report.installed != expected {
        return Err(format!(
            "Manifest completion gate failed for app {app_id}: expected {expected} files, installed {}",
            report.installed
        ));
    }

    // Shared completeness gate: the same "present" definition the resolution
    // used, so a resolved pin can never fail here.
    let remaining =
        crate::manifest::resolver::verify_available(&settings.steam_path, app_id, &pins);
    if !remaining.missing.is_empty() {
        return Err(format!(
            "Manifest completion gate failed for app {app_id}: {} manifest(s) remain missing after install",
            remaining.missing.len()
        ));
    }

    if let Ok(backup) = crate::core::backup::GameBackup::for_app(app_id) {
        let depotcache = PathBuf::from(&settings.steam_path).join("depotcache");
        let _ = backup.backup_referenced_manifests(&content, &depotcache);
    }
    crate::desk_log_info!(
        "hubcap",
        "Explicit game manifest repair complete app_id={} generated={} installed={} elapsed_ms={}",
        app_id,
        report.generated,
        report.installed,
        started.elapsed().as_millis()
    );
    Ok(report)
}

/// Outcome of one Hubcap-driven pin refresh (`refresh_game_pins_from_hubcap`).
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PinRefreshReport {
    pub app_id: u32,
    /// The game was out of scope (no Lua, version-locked, or no managed rows).
    pub skipped: bool,
    /// Managed depot rows diffed against the Hubcap contents.
    pub checked_depots: usize,
    /// Manifests staged into depotcache because they were missing locally.
    pub staged: usize,
    /// Commented pins rewritten to the newer GIDs.
    pub realigned: usize,
}

/// Diffs one game's Lua pins against the manifests Hubcap currently packages
/// (FREE `/manifest/{app_id}/contents` endpoint — no ZIP download and no
/// generation quota spent on the check itself) and, for every managed depot
/// whose current GID differs from the Lua's:
///   1. stages the manifest locally (local-first; only genuinely missing
///      files are generated through the authenticated Hubcap pipeline), then
///   2. rewrites the informational (commented) pin in the Lua.
///
/// This keeps "Disable updates" honest for updates-ON games: the pins the
/// toggle reactivates always point at the newest version Aether knows —
/// Steam's installed truth first (newest depotcache file per depot), Hubcap's
/// packaged view second — never at a stale GID that would trigger a
/// downgrade. Version-locked games (active pins) are never touched, and
/// manifests are always committed BEFORE the Lua rewrite because the Lua is
/// the DLL hot-reload trigger. Hubcap-only by design: no SteamDB/Depotbox.
pub async fn refresh_game_pins_from_hubcap(
    app: tauri::AppHandle,
    app_id: u32,
) -> Result<PinRefreshReport, String> {
    if app_id == 0 {
        return Err("A valid AppID is required".to_string());
    }
    let settings = SettingsManager::new(&app).load();
    if settings.steam_path.trim().is_empty() {
        return Err("Pin refresh cannot run because the Steam path is empty".to_string());
    }
    if settings.hubcap_api_key.trim().is_empty() {
        return Err("Pin refresh requires a configured Hubcap API key".to_string());
    }

    let editor = LuaManifestPins::new(settings.steam_path.clone(), app_id);
    if !editor.path_exists() {
        return Ok(PinRefreshReport { app_id, skipped: true, ..Default::default() });
    }
    let content = editor.read_lua()?;
    LuaManifestPins::validate_content(&content)?;

    // Local backup pass BEFORE any gate and any HTTP: every manifest the
    // current Lua references (active or commented pins) that exists in
    // depotcache but not yet in AetherData gets archived. This is what makes
    // the backup complete for version-locked games too, and it captures
    // manifests the DLL generated live while Desk was closed. Cheap and
    // idempotent: a non-empty destination is never overwritten, so the
    // 15-minute lane cadence costs nothing once the tree is aligned.
    if let Ok(backup) = crate::core::backup::GameBackup::for_app(app_id) {
        let depotcache = PathBuf::from(&settings.steam_path).join("depotcache");
        let _ = backup.backup_referenced_manifests(&content, &depotcache);
    }

    if !editor.updates_are_enabled()? {
        crate::desk_log_debug!(
            "hubcap",
            "Pin refresh skipped app_id={}: version-locked Lua (active pins are intentional)",
            app_id
        );
        return Ok(PinRefreshReport { app_id, skipped: true, ..Default::default() });
    }
    let rows = LuaManifestPins::rows_for_manifest_sync(&content);
    if rows.is_empty() {
        return Ok(PinRefreshReport { app_id, skipped: true, ..Default::default() });
    }

    let started = Instant::now();
    let client = HubcapClient::new(settings.hubcap_api_key.clone());
    if !client.validate_api_key().await? {
        return Err("Hubcap API key is not valid or is not allowed to make requests.".to_string());
    }
    let contents = client.get_app_contents(app_id).await?;
    if contents.manifests.is_empty() {
        crate::desk_log_debug!(
            "hubcap",
            "Pin refresh app_id={}: Hubcap packages no manifests for this app",
            app_id
        );
        return Ok(PinRefreshReport {
            app_id,
            skipped: false,
            checked_depots: rows.len(),
            ..Default::default()
        });
    }

    // Per-depot target selection. Local evidence wins: the newest manifest
    // file in depotcache is what Steam actually installed (e.g. an update ran
    // while Desk was closed). Only when there is no newer local GID does the
    // Hubcap packaged view get adopted. Manifest GIDs are NOT chronologically
    // ordered, so "different from the pin" is the only available signal — and
    // for an updates-ON game both sources only ever move forward with builds.
    let mut targets: Vec<DepotManifestPin> = Vec::new();
    for row in &rows {
        let local =
            crate::core::hubcap_update_monitor::latest_installed_gid(&settings.steam_path, row.app_id)
                .filter(|gid| *gid != row.manifest_id);
        let candidate = match local {
            Some(gid) => Some(gid),
            None => contents
                .manifests
                .get(&row.app_id)
                .cloned()
                .filter(|gid| *gid != row.manifest_id),
        };
        if let Some(manifest_id) = candidate {
            targets.push(DepotManifestPin {
                depot_id: row.app_id,
                manifest_id,
            });
        }
    }
    if targets.is_empty() {
        return Ok(PinRefreshReport {
            app_id,
            skipped: false,
            checked_depots: rows.len(),
            ..Default::default()
        });
    }

    // Stage the manifests BEFORE any Lua edit: the Lua is the DLL hot-reload
    // trigger and must only change once every referenced file is on disk.
    let resolution =
        crate::manifest::resolver::resolve(crate::manifest::resolver::ManifestRequest {
            steam_path: settings.steam_path.clone(),
            app_id,
            pins: targets.clone(),
            generation: crate::manifest::resolver::Generation::PreValidatedKey(
                settings.hubcap_api_key.clone(),
            ),
        })
        .await?;
    if !resolution.is_complete() {
        return Err(format!(
            "Hubcap could not provide {} manifest(s) for the pin refresh of app {app_id}; the Lua was not modified",
            resolution.missing.len()
        ));
    }
    let mut staged = 0usize;
    if !resolution.generated.is_empty() {
        let expected = resolution.generated.len();
        staged =
            SteamCompat::new(settings.steam_path.clone()).install_manifest_files(&resolution.generated)?;
        if staged != expected {
            return Err(format!(
                "Pin refresh gate failed for app {app_id}: expected {expected} staged manifests, installed {staged}"
            ));
        }
    }
    let remaining =
        crate::manifest::resolver::verify_available(&settings.steam_path, app_id, &targets);
    if !remaining.missing.is_empty() {
        return Err(format!(
            "Pin refresh gate failed for app {app_id}: {} manifest(s) are still missing after staging",
            remaining.missing.len()
        ));
    }

    // Preserve the pre-change Lua exactly like the other pin pipelines do.
    if let Ok(backup) = crate::core::backup::GameBackup::for_app(app_id) {
        let _ = backup.store_history_version(app_id, content.as_bytes());
    }
    let realigned = editor.realign_commented_pins(&targets)?;

    // Archive the NEW Lua too (every Lua change must land in AetherData) and
    // copy the freshly staged manifests so a future uninstall can restore
    // this version.
    if let (Ok(final_lua), Ok(backup)) =
        (editor.read_lua(), crate::core::backup::GameBackup::for_app(app_id))
    {
        let _ = backup.store_history_version(app_id, final_lua.as_bytes());
        let depotcache = PathBuf::from(&settings.steam_path).join("depotcache");
        let _ = backup.backup_referenced_manifests(&final_lua, &depotcache);
    }

    crate::core::library_events::notify_lua_changed(
        &app,
        crate::core::library_events::LibraryChangeOrigin::Versioning,
        [app_id],
    );
    crate::desk_log_info!(
        "hubcap",
        "Pin refresh complete app_id={} checked_depots={} targets={} staged={} realigned={} elapsed_ms={}",
        app_id,
        rows.len(),
        targets.len(),
        staged,
        realigned,
        started.elapsed().as_millis()
    );
    Ok(PinRefreshReport {
        app_id,
        skipped: false,
        checked_depots: rows.len(),
        staged,
        realigned,
    })
}
