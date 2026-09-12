use serde::Serialize;
use std::path::PathBuf;
use std::time::Instant;

use crate::core::settings::SettingsManager;
use crate::manifest::pins::{pins_from_rows, DepotManifestPin, LuaManifestPins};
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
