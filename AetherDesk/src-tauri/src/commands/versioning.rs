use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::task::JoinSet;

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
/// The small rolling task window prevents a large DLC set from creating an
/// unbounded request burst while allowing independent Hubcap generations to
/// run in parallel.
pub(crate) async fn generate_missing_manifests(
    client: HubcapClient,
    missing: Vec<DepotManifestPin>,
) -> Result<Vec<ManifestPackageFile>, String> {
    let mut tasks = JoinSet::new();
    let mut pending = missing.into_iter();
    let concurrency = 8usize;

    for _ in 0..concurrency {
        let Some(pin) = pending.next() else { break };
        let worker = client.clone();
        tasks.spawn(async move {
            let result = match pin.manifest_id.parse::<u64>() {
                Ok(gid) => worker.generate_manifest(pin.depot_id, gid).await,
                Err(_) => Err(format!("Invalid manifest GID for depot {}", pin.depot_id)),
            };
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
                let result = match next_pin.manifest_id.parse::<u64>() {
                    Ok(gid) => worker.generate_manifest(next_pin.depot_id, gid).await,
                    Err(_) => Err(format!("Invalid manifest GID for depot {}", next_pin.depot_id)),
                };
                (next_pin, result)
            });
        }
    }
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
    crate::desk_log_info!(
        "versioning",
        "Applying build {} to {}",
        build_id,
        crate::core::logger::format_appid(app_id)
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

    let service = build_service(&app);
    let pins = match service
        .resolve_snapshot_pins(app_id, build_id, &depot_ids)
        .await
    {
        Ok(pins) => pins,
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

    let generated_manifests = if local_missing.is_empty() {
        Vec::new()
    } else {
        let hubcap_key = crate::core::settings::SettingsManager::new(&app)
            .load()
            .hubcap_api_key;
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
        generate_missing_manifests(HubcapClient::new(hubcap_key), local_missing).await?
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
                "Build {} applied for {}: {} pin(s) written from the reconstructed snapshot, acf_synced={}",
                build_id,
                crate::core::logger::format_appid(app_id),
                report.applied_pins,
                report.acf_synced_now
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

