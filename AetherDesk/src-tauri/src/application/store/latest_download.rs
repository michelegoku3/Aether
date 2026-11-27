//! Latest-version provider download use cases.
//!
//! Flow: validate, acquire/complete a provider package, publish it through the
//! shared installer, then report without exposing provider details to IPC.

use super::concurrency::{acquire_latest_package, acquire_server_job, ServerJobGuard};
use super::installation::install_standard_package;
use super::model::{AuthenticatedDownloadRequest, LuaToolsDownloadRequest};
use super::package_completion::{complete_hubcap_package, download_complete_luatools_package};
use super::validation::{validate_authenticated_download, validate_steam_download_path};
use crate::core::game_mutations::MutationPlan;
use crate::manifest::package::ManifestPackage;
use crate::providers::depotbox::DepotBoxClient;
use crate::providers::hubcap::HubcapClient;
use crate::providers::ryuu::RyuuClient;

pub async fn trigger_hubcap_download(
    app: &tauri::AppHandle,
    request: AuthenticatedDownloadRequest,
) -> Result<String, String> {
    let app_id = request.download.app_id;
    let steam_path = request.download.steam_path;
    let api_key = request.api_key;
    let plan = MutationPlan::prepare(
        std::path::Path::new(&steam_path),
        app_id,
        "trigger_hubcap_download",
    )?;
    if let Err(error) =
        validate_authenticated_download(&api_key, &steam_path, "call Hubcap Manifest")
    {
        crate::desk_log_error!(
            "store",
            "Download failed for {}: {}",
            crate::core::logger::format_appid(app_id),
            error
        );
        return Err(error);
    }

    let _package_guard = acquire_latest_package(app_id).await?;
    crate::desk_log_info!(
        "store",
        "Triggering download for {} (source: hubcap)",
        crate::core::logger::format_appid(app_id)
    );

    let result = async {
        let hubcap = HubcapClient::new(&api_key);
        if !hubcap.validate_api_key().await? {
            return Err(
                "Hubcap API key is not valid or is not allowed to make requests.".to_string(),
            );
        }
        let mut package = hubcap.download_lua_package(app_id).await?;
        complete_hubcap_package(app_id, &steam_path, &mut package, &api_key).await?;
        install_standard_package(app, app_id, &steam_path, package, "Hubcap", plan).await
    }
    .await;

    match &result {
        Ok(message) => crate::desk_log_info!(
            "store",
            "Successfully completed download for {}: {}",
            crate::core::logger::format_appid(app_id),
            message
        ),
        Err(error) => crate::desk_log_error!(
            "store",
            "Download failed for {} (source: hubcap): {}",
            crate::core::logger::format_appid(app_id),
            error
        ),
    }
    result
}

pub async fn trigger_ryuu_download(
    app: &tauri::AppHandle,
    request: AuthenticatedDownloadRequest,
) -> Result<String, String> {
    let app_id = request.download.app_id;
    let steam_path = request.download.steam_path;
    let api_key = request.api_key;
    let plan = MutationPlan::prepare(
        std::path::Path::new(&steam_path),
        app_id,
        "trigger_ryuu_download",
    )?;
    if let Err(error) = validate_authenticated_download(&api_key, &steam_path, "call Ryuu") {
        crate::desk_log_error!(
            "store",
            "Ryuu download failed for {}: {}",
            crate::core::logger::format_appid(app_id),
            error
        );
        return Err(error);
    }

    crate::desk_log_info!(
        "store",
        "Triggering Ryuu download for {}",
        crate::core::logger::format_appid(app_id)
    );
    let result = async {
        let package = RyuuClient::new(api_key)
            .download_lua_package(app_id)
            .await?;
        install_standard_package(app, app_id, &steam_path, package, "Ryuu", plan).await
    }
    .await;
    match &result {
        Ok(message) => crate::desk_log_info!(
            "store",
            "Successfully completed Ryuu download for {}: {}",
            crate::core::logger::format_appid(app_id),
            message
        ),
        Err(error) => crate::desk_log_error!(
            "store",
            "Ryuu download failed for {}: {}",
            crate::core::logger::format_appid(app_id),
            error
        ),
    }
    result
}

/// Starts one DepotBox job and returns the package together with the per-AppID
/// guard. The guard is the caller's to hold until publication finishes, so two
/// downloads of the same game can never interleave; the process-wide provider
/// lane is released as soon as the server owns the build.
pub(super) async fn acquire_depotbox_package(
    app_id: u32,
    api_key: &str,
) -> Result<(ServerJobGuard, ManifestPackage), String> {
    let client = DepotBoxClient::new(api_key);
    let mut job_gate = acquire_server_job(app_id).await?;
    let job = client.start_job(app_id).await?;
    job_gate.release_provider_lane();
    let package = client.wait_for_package(&job).await?;
    Ok((job_gate, package))
}

/// Completes a DepotBox package local-first, falling back to Hubcap manifest
/// generation with the key already configured in Settings. DepotBox is not a
/// manifest generator: without a Hubcap key it cannot fill gaps by itself.
pub(super) async fn complete_depotbox_package(
    app: &tauri::AppHandle,
    app_id: u32,
    steam_path: &str,
    package: &mut ManifestPackage,
) -> Result<(), String> {
    let bundled = package.manifest_files.len();
    let hubcap_key = crate::core::settings::load_settings(app)
        .hubcap_api_key
        .trim()
        .to_string();
    if hubcap_key.is_empty() {
        crate::desk_log_info!(
            "store",
            "DepotBox package for {}: no Hubcap key configured, completion uses local manifests only",
            crate::core::logger::format_appid(app_id)
        );
    }
    complete_hubcap_package(app_id, steam_path, package, &hubcap_key).await?;
    crate::desk_log_info!(
        "store",
        "DepotBox package for {} complete: {} bundled manifest(s), {} added from local/generated sources",
        crate::core::logger::format_appid(app_id),
        bundled,
        package.manifest_files.len() - bundled
    );
    Ok(())
}

pub async fn trigger_depotbox_download(
    app: &tauri::AppHandle,
    request: AuthenticatedDownloadRequest,
) -> Result<String, String> {
    let app_id = request.download.app_id;
    let steam_path = request.download.steam_path;
    let api_key = request.api_key;
    if let Err(error) = validate_authenticated_download(&api_key, &steam_path, "call DepotBox") {
        crate::desk_log_error!(
            "store",
            "DepotBox download failed for {}: {}",
            crate::core::logger::format_appid(app_id),
            error
        );
        return Err(error);
    }
    crate::desk_log_info!(
        "store",
        "Triggering DepotBox download for {} (source: depotbox)",
        crate::core::logger::format_appid(app_id)
    );

    let result = async {
        // The mutation plan is prepared after the download: a remote build can
        // take minutes, and a plan snapshotted before it would be more likely
        // to be stale by publication time.
        let (_job_guard, mut package) = acquire_depotbox_package(app_id, &api_key).await?;
        complete_depotbox_package(app, app_id, &steam_path, &mut package).await?;
        let plan = MutationPlan::prepare(
            std::path::Path::new(&steam_path),
            app_id,
            "trigger_depotbox_download",
        )?;
        install_standard_package(app, app_id, &steam_path, package, "DepotBox", plan).await
    }
    .await;

    match &result {
        Ok(message) => crate::desk_log_info!(
            "store",
            "Successfully completed DepotBox download for {}: {}",
            crate::core::logger::format_appid(app_id),
            message
        ),
        Err(error) => crate::desk_log_error!(
            "store",
            "DepotBox download failed for {} (source: depotbox): {}",
            crate::core::logger::format_appid(app_id),
            error
        ),
    }
    result
}

pub async fn trigger_luatools_download(
    app: &tauri::AppHandle,
    request: LuaToolsDownloadRequest,
) -> Result<String, String> {
    let app_id = request.download.app_id;
    let steam_path = request.download.steam_path;
    let plan = MutationPlan::prepare(
        std::path::Path::new(&steam_path),
        app_id,
        "trigger_luatools_download",
    )?;
    validate_steam_download_path(&steam_path)?;
    crate::desk_log_info!(
        "store",
        "Triggering authenticated LuaTools download for {}",
        crate::core::logger::format_appid(app_id)
    );
    let package =
        download_complete_luatools_package(app, app_id, &steam_path, request.game_name.as_deref())
            .await?;
    install_standard_package(app, app_id, &steam_path, package, "LuaTools", plan).await
}
