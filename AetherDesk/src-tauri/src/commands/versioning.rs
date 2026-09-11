use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::task::JoinSet;
use std::time::Instant;

use crate::manifest::package::ManifestPackageFile;
use crate::manifest::pins::DepotManifestPin;
use crate::providers::hubcap::HubcapClient;

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

/// Fetches only the exact manifest pins that are absent from Steam/AetherData.
/// A single shared provider lane prevents a large DLC set, Workshop work, and
/// package completion from creating an unbounded request burst.
pub(crate) async fn generate_missing_manifests(
    client: HubcapClient,
    missing: Vec<DepotManifestPin>,
) -> Result<Vec<ManifestPackageFile>, String> {
    let started = Instant::now();
    crate::desk_log_info!("versioning", "Manifest generation batch start missing={}", missing.len());
    let mut tasks = JoinSet::new();
    let mut pending = missing.into_iter();
    // Hubcap's generation endpoints are rate limited independently of the
    // daily quota. Keep a user-triggered multi-depot operation observable and
    // bounded instead of turning one click into a burst of parallel requests.
    // Keep this lane serial. Hubcap generation is quota/rate limited and the
    // provider scheduler already deduplicates requests across game, Workshop,
    // and store flows; a second task would only create avoidable 429 pressure.
    let concurrency = 1usize;

    for _ in 0..concurrency {
        let Some(pin) = pending.next() else { break };
        let worker = client.clone();
        tasks.spawn(async move {
            crate::desk_log_info!(
                "versioning",
                "Manifest generation start depot_id={} manifest_id={}",
                pin.depot_id,
                pin.manifest_id
            );
            let result = match pin.manifest_id.parse::<u64>() {
                Ok(gid) => worker.generate_manifest(pin.depot_id, gid).await,
                Err(_) => Err(format!("Invalid manifest GID for depot {}", pin.depot_id)),
            };
            crate::desk_log_info!(
                "versioning",
                "Manifest generation complete depot_id={} manifest_id={} success={}",
                pin.depot_id,
                pin.manifest_id,
                result.is_ok()
            );
            (pin, result)
        });
    }

    let mut files = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        let (pin, result) = joined
            .map_err(|error| format!("Hubcap manifest task failed: {error}"))?;
        let bytes = result.map_err(|error| {
            format!(
                "Hubcap could not generate manifest {}_{}: {}",
                pin.depot_id, pin.manifest_id, error
            )
        })?;
        files.push(ManifestPackageFile {
            file_name: format!("{}_{}.manifest", pin.depot_id, pin.manifest_id),
            bytes,
        });

        if let Some(next_pin) = pending.next() {
            let worker = client.clone();
            tasks.spawn(async move {
                crate::desk_log_info!(
                    "versioning",
                    "Manifest generation start depot_id={} manifest_id={}",
                    next_pin.depot_id,
                    next_pin.manifest_id
                );
                let result = match next_pin.manifest_id.parse::<u64>() {
                    Ok(gid) => worker.generate_manifest(next_pin.depot_id, gid).await,
                    Err(_) => Err(format!("Invalid manifest GID for depot {}", next_pin.depot_id)),
                };
                crate::desk_log_info!(
                    "versioning",
                    "Manifest generation complete depot_id={} manifest_id={} success={}",
                    next_pin.depot_id,
                    next_pin.manifest_id,
                    result.is_ok()
                );
                (next_pin, result)
            });
        }
    }
    crate::desk_log_info!(
        "versioning",
        "Manifest generation batch complete generated={} elapsed_ms={}",
        files.len(),
        started.elapsed().as_millis()
    );
    Ok(files)
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
    let local_steam_path = steam_path.clone();
    let local_pins = pins.clone();
    let local_missing = tauri::async_runtime::spawn_blocking(move || {
        crate::versioning::apply::prepare_local_manifests(&local_steam_path, app_id, &local_pins)
    })
    .await
    .map_err(|error| format!("Local manifest preparation failed: {error}"))??;
    crate::desk_log_info!(
        "versioning",
        "Local manifest preparation complete app_id={} build_id={} pins={} missing={}",
        app_id,
        build_id,
        pins.len(),
        local_missing.len()
    );

    let generated_manifests = if local_missing.is_empty() {
        crate::desk_log_info!("versioning", "No Hubcap generation required; all pinned manifests are local");
        Vec::new()
    } else {
        let hubcap_key = crate::core::settings::SettingsManager::new(&app)
            .load()
            .hubcap_api_key;
        crate::desk_log_info!(
            "versioning",
            "Missing manifest recovery required count={} hubcap_key_configured={}",
            local_missing.len(),
            !hubcap_key.trim().is_empty()
        );
        if hubcap_key.trim().is_empty() {
            let missing = local_missing
                .iter()
                .map(|pin| format!("{}:{}", pin.depot_id, pin.manifest_id))
                .collect::<Vec<_>>()
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
                message: format!("Generating {} missing manifest(s) with Hubcap...", local_missing.len()),
            },
        );
        let hubcap = HubcapClient::new(hubcap_key);
        if !hubcap.validate_api_key().await? {
            return Err("Hubcap API key is not valid or is not allowed to make requests.".to_string());
        }
        let generated = generate_missing_manifests(hubcap, local_missing).await?;
        crate::desk_log_info!(
            "versioning",
            "Missing manifest recovery complete generated={}",
            generated.len()
        );
        generated
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

