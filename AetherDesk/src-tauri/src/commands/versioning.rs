use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::util::validation::validate_steam_path;
use crate::versioning::model::{ApplyVersionReport, BuildInfo, BuildPreview, SavedBuild};
use crate::versioning::queue::PendingAcfEdit;
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
    let token =
        crate::versioning::sources::resolve_build_details_token(Some(&settings.build_details_token));
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

/// Read-only plan of what applying a build would change.
#[tauri::command]
pub async fn get_build_preview(
    app: AppHandle,
    app_id: u32,
    build_id: u64,
    steam_path: String,
) -> Result<BuildPreview, String> {
    validate_steam_path(&steam_path)?;
    validate_app_build(app_id, build_id)?;
    crate::desk_log_info!(
        "versioning",
        "Previewing build {} for {}",
        build_id,
        crate::core::logger::format_appid(app_id)
    );
    match build_service(&app)
        .preview_build(app_id, build_id, &steam_path)
        .await
    {
        Ok(preview) => {
            crate::desk_log_info!(
                "versioning",
                "Preview ready for build {}: {} matching pin(s), {} depot(s) to disable",
                build_id,
                preview.matching_pins.len(),
                preview.missing_depots.len()
            );
            Ok(preview)
        }
        Err(err) => {
            crate::desk_log_error!(
                "versioning",
                "Preview of build {} for {} failed: {}",
                build_id,
                crate::core::logger::format_appid(app_id),
                err
            );
            Err(err.into())
        }
    }
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

    let service = build_service(&app);
    let pins = match service.resolve_pins(build_id).await {
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

    // ACF lives in the active library (falls back to the Steam root folder).
    let settings = crate::core::settings::SettingsManager::new(&app).load();
    let library_path = if settings.active_library.trim().is_empty() {
        steam_path.clone()
    } else {
        settings.active_library.clone()
    };

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
        service.apply_build_sync(app_id, build_id, &steam_path, &library_path, &pins, &progress)
    })
    .await
    .map_err(|e| format!("Version task failed: {e}"))?;

    match pipeline_result {
        Ok(report) => {
            crate::desk_log_info!(
                "versioning",
                "Build {} applied for {}: {} pin(s) written (absent-from-diff depots left unchanged), acf_synced={}, acf_queued={}",
                build_id,
                crate::core::logger::format_appid(app_id),
                report.applied_pins,
                report.acf_synced_now,
                report.acf_queued
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

/// ACF edits waiting for the game to be downloaded / Steam to release the file.
#[tauri::command]
pub fn get_pending_version_edits(_app: AppHandle) -> Result<Vec<PendingAcfEdit>, String> {
    Ok(crate::versioning::queue::list())
}
