//! Latest-version provider download use cases.
//!
//! Flow: validate, acquire/complete a provider package, publish it through the
//! shared installer, then report without exposing provider details to IPC.

use super::concurrency::acquire_latest_package;
use super::installation::install_standard_package;
use super::model::{AuthenticatedDownloadRequest, LuaToolsDownloadRequest};
use super::package_completion::{complete_hubcap_package, download_complete_luatools_package};
use super::source::HubcapSource;
use super::validation::{validate_authenticated_download, validate_steam_download_path};
use crate::core::game_mutations::MutationPlan;
use crate::providers::hubcap::HubcapClient;
use crate::providers::ryuu::RyuuClient;

pub async fn trigger_hubcap_download(
    app: &tauri::AppHandle,
    request: AuthenticatedDownloadRequest,
) -> Result<String, String> {
    let app_id = request.download.app_id;
    let steam_path = request.download.steam_path;
    let api_key = request.api_key;
    let source = HubcapSource::from_api_key(&api_key);
    let plan = MutationPlan::prepare(
        std::path::Path::new(&steam_path),
        app_id,
        "trigger_hubcap_download",
    )?;
    if let Err(error) = validate_authenticated_download(&api_key, &steam_path, "call Hubcap Manifest") {
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
        "Triggering download for {} (source: {})",
        crate::core::logger::format_appid(app_id),
        source.log_name()
    );

    let result = async {
        let mut package = match source {
            HubcapSource::Oureveryday => {
                crate::providers::oureveryday::OureverydayClient::new()
                    .download_lua_package(app_id)
                    .await?
            }
            HubcapSource::Hubcap => {
                let hubcap = HubcapClient::new(&api_key);
                if !hubcap.validate_api_key().await? {
                    return Err(
                        "Hubcap API key is not valid or is not allowed to make requests."
                            .to_string(),
                    );
                }
                hubcap.download_lua_package(app_id).await?
            }
        };
        if source == HubcapSource::Hubcap {
            complete_hubcap_package(app_id, &steam_path, &mut package, &api_key).await?;
        }
        install_standard_package(
            app,
            app_id,
            &steam_path,
            package,
            source.display_name(),
            plan,
        )
        .await
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
            "Download failed for {} (source: {}): {}",
            crate::core::logger::format_appid(app_id),
            source.log_name(),
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
        let package = RyuuClient::new(api_key).download_lua_package(app_id).await?;
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
    let package = download_complete_luatools_package(
        app,
        app_id,
        &steam_path,
        request.game_name.as_deref(),
    )
    .await?;
    install_standard_package(app, app_id, &steam_path, package, "LuaTools", plan).await
}
