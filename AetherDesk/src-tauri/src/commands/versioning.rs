use serde::Serialize;
use tauri::{AppHandle, Emitter};
use std::time::Instant;

use crate::manifest::pins::LuaManifestPins;
use crate::util::validation::validate_steam_path;
use crate::versioning::model::{ApplyVersionReport, BuildInfo, SavedBuild};
use crate::versioning::service::VersionService;

/// Progress event emitted while `apply_game_version` runs (and later by the
/// background ACF retry worker). Payload mirrors the model types in camelCase.
const PROGRESS_EVENT: &str = "versioning://progress";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VersionProgressEvent {
    app_id: u32,
    build_id: u64,
    step: u8,
    message: String,
}

fn build_service(app: &AppHandle) -> VersionService {
    let settings = crate::core::settings::SettingsManager::new(app).load();
    let token = crate::versioning::sources::resolve_build_details_token(Some(
        &settings.build_details_token,
    ));
    VersionService::with_token(token)
}

fn validate_app_build(app_id: u32, build_id: u64) -> Result<(), String> {
    if app_id == 0 {
        return Err("A valid Steam App ID is required".to_string());
    }
    // Steam build IDs are always 7+ digits.
    if build_id < 1_000_000 {
        return Err("A valid Build ID (7+ digits) is required".to_string());
    }
    Ok(())
}

/// All published builds of a game, newest first (cached 24 h).
#[tauri::command]
pub async fn get_game_builds(app: AppHandle, app_id: u32) -> Result<Vec<BuildInfo>, String> {
    if app_id == 0 {
        return Err("A valid Steam App ID is required".to_string());
    }
    crate::desk_log_info!(
        "versioning",
        "Fetching build history for {}",
        crate::core::logger::format_appid(app_id)
    );
    build_service(&app)
        .list_builds(app_id)
        .await
        .map_err(String::from)
}

/// Applies a build: pins the Lua, syncs the ACF (or queues the edit) and
/// reports the real outcome. Emits `versioning://progress` along the way.
#[tauri::command]
pub async fn apply_game_version(
    app: AppHandle,
    app_id: u32,
    build_id: u64,
    steam_path: String,
) -> Result<ApplyVersionReport, String> {
    validate_steam_path(&steam_path)?;
    validate_app_build(app_id, build_id)?;
    let started = Instant::now();
    crate::desk_log_info!(
        "versioning",
        "Apply start app_id={} build_id={} steam_path_configured={}",
        app_id,
        build_id,
        !steam_path.trim().is_empty()
    );

    // The build lookup is the only slow phase: report it to the UI so the
    // user never stares at a silent spinner.
    let _ = app.emit(
        PROGRESS_EVENT,
        VersionProgressEvent {
            app_id,
            build_id,
            step: 5,
            message: "Resolving build manifests...".to_string(),
        },
    );

    // Read the complete Lua depot set before resolving remote data. A build's
    // details contain only depots changed by that patch; the service needs the
    // full desired set to reconstruct every depot's state at the target build.
    let lua = LuaManifestPins::new(steam_path.clone(), app_id);
    let depot_ids = lua.depot_ids_from_file().map_err(|err| {
        if !lua.path_exists() {
            crate::versioning::error::VersionError::LuaMissing(lua.lua_path().display().to_string())
                .to_string()
        } else {
            crate::versioning::error::VersionError::Lua(err).to_string()
        }
    })?;
    crate::desk_log_info!(
        "versioning",
        "Build snapshot input resolved app_id={} build_id={} depot_count={}",
        app_id,
        build_id,
        depot_ids.len()
    );

    let service = build_service(&app);
    let pins = match service
        .resolve_snapshot_pins(app_id, build_id, &depot_ids)
        .await
    {
        Ok(pins) => {
            crate::desk_log_info!(
                "versioning",
                "Build snapshot pins resolved app_id={} build_id={} pin_count={}",
                app_id,
                build_id,
                pins.len()
            );
            pins
        }
        Err(err) => {
            crate::desk_log_error!(
                "versioning",
                "Apply of build {} for {} failed during pin resolution: {}",
                build_id,
                crate::core::logger::format_appid(app_id),
                err
            );
            return Err(err.into());
        }
    };

    // Exact local lookup is deliberately first. Backup hits are restored into
    // Steam/depotcache without spending Hubcap quota; only the remaining pins
    // enter the authenticated generation scheduler.
    let local_resolution = crate::manifest::resolver::resolve(
        crate::manifest::resolver::ManifestRequest {
            steam_path: steam_path.clone(),
            app_id,
            pins: pins.clone(),
            generation: crate::manifest::resolver::Generation::LocalOnly,
        },
    )
    .await?;
    crate::desk_log_info!(
        "versioning",
        "Local manifest preparation complete app_id={} build_id={} pins={} missing={}",
        app_id,
        build_id,
        pins.len(),
        local_resolution.missing.len()
    );

    let generated_manifests = if local_resolution.missing.is_empty() {
        crate::desk_log_info!("versioning", "No Hubcap generation required; all pinned manifests are local");
        Vec::new()
    } else {
        let hubcap_key = crate::core::settings::SettingsManager::new(&app)
            .load()
            .hubcap_api_key;
        crate::desk_log_info!(
            "versioning",
            "Missing manifest recovery required count={} hubcap_key_configured={}",
            local_resolution.missing.len(),
            !hubcap_key.trim().is_empty()
        );
        if hubcap_key.trim().is_empty() {
            let missing = crate::manifest::resolver::missing_labels(&local_resolution.missing)
                .join(", ");
            return Err(format!(
                "This version needs manifest(s) that are not stored locally: {missing}. Configure and validate a Hubcap API key before switching versions; no unauthenticated request-code fallback is used."
            ));
        }
        let _ = app.emit(
            PROGRESS_EVENT,
            VersionProgressEvent {
                app_id,
                build_id,
                step: 20,
                message: format!("Generating {} missing manifest(s) with Hubcap...", local_resolution.missing.len()),
            },
        );
        let recovered = crate::manifest::resolver::resolve(
            crate::manifest::resolver::ManifestRequest {
                steam_path: steam_path.clone(),
                app_id,
                pins: local_resolution.missing,
                generation: crate::manifest::resolver::Generation::SettingsKey(hubcap_key),
            },
        )
        .await?
        .generated;
        crate::desk_log_info!(
            "versioning",
            "Missing manifest recovery complete generated={}",
            recovered.len()
        );
        recovered
    };

    // ACF lives in the Steam root library. (`steam_path` was strict-validated
    // at command entry; normalize the raw value before reuse.)
    let library_path = crate::steam::resolve::normalize_steam_path(&steam_path);

    let pins_for_apply = pins;
    let generated_for_apply = generated_manifests;
    let handle = app.clone();
    let pipeline_result = tauri::async_runtime::spawn_blocking(move || {
        let progress = |step: u8, message: &str| {
            let _ = handle.emit(
                PROGRESS_EVENT,
                VersionProgressEvent {
                    app_id,
                    build_id,
                    step,
                    message: message.to_string(),
                },
            );
        };
        service.apply_build_sync(
            app_id,
            build_id,
            &steam_path,
            &library_path,
            &pins_for_apply,
            &generated_for_apply,
            &progress,
        )
    })
    .await
    .map_err(|e| format!("Version task failed: {e}"))?;

    match pipeline_result {
        Ok(report) => {
            crate::desk_log_info!(
                "versioning",
                "Apply complete app_id={} build_id={} applied_pins={} manifests_found={} manifests_missing={} acf_synced={} elapsed_ms={}",
                app_id,
                build_id,
                report.applied_pins,
                report.manifests_found,
                report.manifests_missing.len(),
                report.acf_synced_now,
                started.elapsed().as_millis()
            );
            crate::core::library_events::notify_lua_changed(
                &app,
                crate::core::library_events::LibraryChangeOrigin::Versioning,
                [app_id],
            );
            Ok(report)
        }
        Err(err) => {
            crate::desk_log_error!(
                "versioning",
                "Apply of build {} for {} failed: {}",
                build_id,
                crate::core::logger::format_appid(app_id),
                err
            );
            Err(err.into())
        }
    }
}

/// Builds the user bookmarked for this game.
#[tauri::command]
pub fn get_saved_builds(app: AppHandle, app_id: u32) -> Result<Vec<SavedBuild>, String> {
    Ok(build_service(&app).list_saved(app_id))
}

#[tauri::command]
pub fn save_build(
    app: AppHandle,
    app_id: u32,
    build_id: u64,
    date: String,
    title: String,
) -> Result<SavedBuild, String> {
    validate_app_build(app_id, build_id)?;
    crate::desk_log_info!(
        "versioning",
        "Saving build {} for {}",
        build_id,
        crate::core::logger::format_appid(app_id)
    );
    build_service(&app)
        .save_build(app_id, build_id, date, title)
        .map_err(String::from)
}

#[tauri::command]
pub fn remove_saved_build(app: AppHandle, app_id: u32, build_id: u64) -> Result<(), String> {
    validate_app_build(app_id, build_id)?;
    build_service(&app)
        .remove_saved(app_id, build_id)
        .map_err(String::from)
}

