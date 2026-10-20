use serde::Serialize;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::manifest::package::ManifestPackageFile;
use crate::manifest::pins::{pins_from_rows, DepotManifestPin, LuaManifestPins};
use crate::manifest::resolver::{
    exact_local_manifest_path, manifest_file_name, missing_labels, Generation, ManifestRequest,
};
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
    /// 1-based line of the malformed pin that stops this game's Lua from
    /// loading, when there is one. Set with `skipped`: the UI then tells the
    /// user to repair that line instead of showing a silent no-op.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_pin_line: Option<usize>,
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

    let settings = crate::core::settings::SettingsManager::new(&app).try_load()?;
    if settings.steam_path.trim().is_empty() {
        return Err("Manifest repair cannot run because the Steam path is empty".to_string());
    }

    let plan = crate::core::game_mutations::MutationPlan::prepare(
        std::path::Path::new(&settings.steam_path),
        app_id,
        "manifest-repair",
    )?;
    let started = Instant::now();
    let lua = LuaManifestPins::new(settings.steam_path.clone(), app_id);
    let content = std::fs::read_to_string(lua.lua_path())
        .map_err(|error| format!("Could not read Lua for app {app_id}: {error}"))?;
    // A malformed pin is a user-fixable condition, not a failure to retry: the
    // repair lane would otherwise burn its whole retry ladder on a file only
    // the user can fix. Skip with the exact line, and let the editor repair it.
    if let Some(issue) = LuaManifestPins::first_active_issue(&content) {
        crate::desk_log_warn!(
            "hubcap",
            "Manifest repair skipped app_id={} : malformed setManifestid at Lua line {} ({}); repair it in the version editor",
            app_id,
            issue.line,
            issue.problem.reason()
        );
        return Ok(GameManifestSyncReport {
            app_id,
            skipped: true,
            invalid_pin_line: Some(issue.line),
            ..Default::default()
        });
    }
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
    let resolution =
        crate::manifest::resolver::resolve(crate::manifest::resolver::ManifestRequest {
            steam_path: settings.steam_path.clone(),
            app_id,
            pins: pins.clone(),
            generation,
        })
        .await?;
    report.missing = resolution.generated.len() + resolution.missing.len();
    if !resolution.is_complete() {
        return Err(
            "A valid authenticated Hubcap API key is required for missing manifests".to_string(),
        );
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

    let _mutation = plan.commit_for(&app).await?;
    report.generated = resolution.generated.len();
    let expected = resolution.generated.len();
    let generated = resolution.generated;
    report.installed = SteamCompat::new(&settings.steam_path).install_manifest_files(&generated)?;
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

// ============================================================================
// Per-game bundle export (Lua + the exact pinned manifests)
// ============================================================================

/// Result of exporting one game's Lua and pinned manifests as a ZIP.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameBundleExportReport {
    /// Absolute path of the ZIP in the user's Downloads folder.
    pub path: String,
    /// Pin rows found in the Lua, commented ones included: with updates
    /// enabled the pins survive as comments and still name the exported
    /// version, so they must be part of the bundle.
    pub pins: usize,
    /// Manifest files written into the ZIP (local plus generated).
    pub manifests_included: usize,
    /// How many of `manifests_included` came from Hubcap.
    pub manifests_generated: usize,
    /// Pins no manifest could be found or generated for, as `depot:gid`.
    pub manifests_missing: Vec<String>,
    /// Why the missing pins could not be generated. The bundle is written
    /// anyway: the Lua and the manifests that do exist are still worth having.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_error: Option<String>,
}

/// Where one exported manifest comes from.
#[derive(Debug, Clone)]
pub(crate) enum BundleSource {
    /// The exact pinned file already on disk (Steam cache or Aether backup).
    Local(PathBuf),
    /// Bytes generated through Hubcap for this export: they go into the ZIP
    /// only, never into `depotcache`, because exporting must not change what
    /// AetherDLL and Steam load next launch.
    Generated(Vec<u8>),
}

#[derive(Debug, Clone)]
pub(crate) struct BundleManifest {
    pub file_name: String,
    pub source: BundleSource,
}

/// What the read-only local pass found: files to copy and pins still absent.
#[derive(Debug, Default)]
pub(crate) struct LocalBundleScan {
    pub local: Vec<BundleManifest>,
    pub missing: Vec<DepotManifestPin>,
}

/// `<Game name>.zip`, sanitized like the folder names local installs use.
pub(crate) fn bundle_zip_name(game_name: &str, app_id: u32) -> String {
    let sanitized: String = game_name
        .chars()
        .filter(|ch| !matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        .collect();
    let sanitized = sanitized.trim().trim_end_matches(['.', ' ']).trim();
    if sanitized.is_empty() {
        format!("app_{app_id}.zip")
    } else {
        format!("{sanitized}.zip")
    }
}

/// Read-only lookup of every pin in Steam's primary and secondary cache and in
/// the per-game AetherData backup. Unlike the repair path it copies nothing:
/// an export has no business writing into Steam's folders.
pub(crate) fn collect_local_manifests(
    steam_path: &str,
    app_id: u32,
    pins: &[DepotManifestPin],
) -> LocalBundleScan {
    let mut scan = LocalBundleScan::default();
    for pin in pins {
        match exact_local_manifest_path(steam_path, app_id, pin) {
            Some(path) => scan.local.push(BundleManifest {
                file_name: manifest_file_name(pin),
                source: BundleSource::Local(path),
            }),
            None => scan.missing.push(pin.clone()),
        }
    }
    scan
}

/// Turns the pins Hubcap answered into bundle entries; every pin it did not
/// answer stays missing and is reported to the user.
pub(crate) fn merge_generated_manifests(
    missing: &[DepotManifestPin],
    generated: &[ManifestPackageFile],
) -> (Vec<BundleManifest>, Vec<DepotManifestPin>) {
    let mut manifests = Vec::new();
    let mut still_missing = Vec::new();
    for pin in missing {
        let file_name = manifest_file_name(pin);
        match generated.iter().find(|file| file.file_name == file_name) {
            Some(file) => manifests.push(BundleManifest {
                file_name,
                source: BundleSource::Generated(file.bytes.clone()),
            }),
            None => still_missing.push(pin.clone()),
        }
    }
    (manifests, still_missing)
}

/// Writes the ZIP atomically enough for an export: a previous bundle with the
/// same name is replaced, so the user never has to wonder which copy is new.
pub(crate) fn write_game_bundle(
    downloads_dir: &Path,
    zip_name: &str,
    lua_entry: &str,
    lua_bytes: &[u8],
    manifests: &[BundleManifest],
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(downloads_dir).map_err(|error| {
        format!(
            "Failed to create Downloads folder {}: {}",
            downloads_dir.display(),
            error
        )
    })?;
    let zip_path = downloads_dir.join(zip_name);
    if zip_path.exists() {
        std::fs::remove_file(&zip_path).map_err(|error| {
            format!(
                "Failed to replace the previous bundle {}: {}",
                zip_path.display(),
                error
            )
        })?;
    }
    let file = std::fs::File::create(&zip_path).map_err(|error| {
        format!(
            "Failed to create ZIP bundle {}: {}",
            zip_path.display(),
            error
        )
    })?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Lua first: it is the file that names the pins the rest of the archive
    // satisfies.
    zip.start_file(lua_entry, options)
        .map_err(|error| format!("Failed to add {lua_entry} to the bundle: {error}"))?;
    zip.write_all(lua_bytes)
        .map_err(|error| format!("Failed to write {lua_entry} into the bundle: {error}"))?;

    for manifest in manifests {
        let file_name = &manifest.file_name;
        let bytes = match &manifest.source {
            BundleSource::Local(path) => std::fs::read(path).map_err(|error| {
                format!(
                    "Failed to read manifest {} for the bundle: {}",
                    path.display(),
                    error
                )
            })?,
            BundleSource::Generated(bytes) => bytes.clone(),
        };
        zip.start_file(file_name, options)
            .map_err(|error| format!("Failed to add {file_name} to the bundle: {error}"))?;
        zip.write_all(&bytes)
            .map_err(|error| format!("Failed to write {file_name} into the bundle: {error}"))?;
    }

    zip.finish()
        .map_err(|error| format!("Failed to finalize {}: {}", zip_path.display(), error))?;
    Ok(zip_path)
}

/// Drops byte-identical duplicate rows. The same depot with another GID is a
/// different pin and stays.
fn unique_pins(pins: Vec<DepotManifestPin>) -> Vec<DepotManifestPin> {
    let mut seen = HashSet::new();
    pins.into_iter()
        .filter(|pin| seen.insert((pin.depot_id, pin.manifest_id.clone())))
        .collect()
}

/// Export one game's Lua plus every pinned manifest as a ZIP in Downloads.
///
/// The Lua is copied byte for byte, including commented pins, and every
/// manifest is resolved by exact `{depot}_{gid}` identity: a newer or older
/// file for the same depot is never substituted. Manifests absent from disk go
/// through the shared Hubcap resolver (the same one the explicit repair uses),
/// so the export matches what "Repair manifests" would install.
#[tauri::command]
pub async fn export_game_bundle(
    app: tauri::AppHandle,
    app_id: u32,
    game_name: String,
) -> Result<GameBundleExportReport, String> {
    if app_id == 0 {
        return Err("A valid AppID is required".to_string());
    }
    let steam_path = crate::commands::command_steam_path(&app)?;
    let settings = crate::core::settings::SettingsManager::new(&app).try_load()?;
    let lua = LuaManifestPins::new(steam_path.clone(), app_id);
    let content = lua
        .read_lua()
        .map_err(|error| format!("Cannot export app {app_id}: {error}"))?;
    let lua_entry = lua
        .lua_path()
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("game.lua")
        .to_string();
    let pins = unique_pins(pins_from_rows(LuaManifestPins::rows_from_content(&content)));

    let scan_pins = pins.clone();
    let scan_steam = steam_path.clone();
    let scan = tauri::async_runtime::spawn_blocking(move || {
        collect_local_manifests(&scan_steam, app_id, &scan_pins)
    })
    .await
    .map_err(|error| format!("Manifest scan failed: {error}"))?;

    let mut manifests = scan.local;
    let mut missing = scan.missing;
    let mut generation_error = None;
    let mut generated_count = 0usize;

    if !missing.is_empty() {
        if settings.hubcap_api_key.trim().is_empty() {
            crate::desk_log_warn!(
                "manifest",
                "Bundle export has no Hubcap key for app_id={} missing={}",
                app_id,
                missing.len()
            );
            generation_error = Some("no Hubcap API key is configured".to_string());
        } else {
            // The shared resolver, restricted to the absent pins: the local
            // pass already collected what exists, so only generation (with its
            // validation and per-pin failure policy) runs here.
            match crate::manifest::resolver::resolve(ManifestRequest {
                steam_path: steam_path.clone(),
                app_id,
                pins: missing.clone(),
                generation: Generation::SettingsKey(settings.hubcap_api_key.clone()),
            })
            .await
            {
                Ok(resolution) => {
                    let (generated, still_missing) =
                        merge_generated_manifests(&missing, &resolution.generated);
                    generated_count = generated.len();
                    manifests.extend(generated);
                    missing = still_missing;
                }
                Err(error) => {
                    crate::desk_log_warn!(
                        "manifest",
                        "Bundle export could not generate missing manifests app_id={} missing={} error={}",
                        app_id,
                        missing.len(),
                        error
                    );
                    generation_error = Some(error);
                }
            }
        }
    }

    let manifests_included = manifests.len();
    let downloads_dir = dirs::download_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let zip_name = bundle_zip_name(&game_name, app_id);
    let lua_bytes = content.into_bytes();
    let path = tauri::async_runtime::spawn_blocking(move || {
        write_game_bundle(
            &downloads_dir,
            &zip_name,
            &lua_entry,
            &lua_bytes,
            &manifests,
        )
    })
    .await
    .map_err(|error| format!("Bundle export task failed: {error}"))??;

    let report = GameBundleExportReport {
        path: path.display().to_string(),
        pins: pins.len(),
        manifests_included,
        manifests_generated: generated_count,
        manifests_missing: missing_labels(&missing),
        generation_error,
    };
    crate::desk_log_info!(
        "manifest",
        "Bundle export complete app_id={} zip={} pins={} included={} generated={} missing={}",
        app_id,
        report.path,
        report.pins,
        report.manifests_included,
        report.manifests_generated,
        report.manifests_missing.len()
    );
    Ok(report)
}
