//! Specific-version provider download use cases.
//!
//! Packages are completed before publication and then flow through one shared
//! installer. Hubcap retains its historical residual-download policy; this
//! refactor intentionally does not change observable behavior.

use super::installation::{install_specific_package, SpecificInstallPolicy};
use super::model::{AuthenticatedDownloadRequest, LuaToolsDownloadRequest};
use super::package_completion::{complete_specific_package, download_complete_luatools_package};
use super::source::HubcapSource;
use super::validation::{validate_authenticated_download, validate_steam_download_path};
use crate::core::game_mutations::MutationPlan;
use crate::manifest::pins::{LuaManifestPins, LuaManifestRow};
use crate::providers::hubcap::HubcapClient;
use crate::providers::ryuu::RyuuClient;

pub async fn prepare_specific_version_download(
    app: &tauri::AppHandle,
    request: AuthenticatedDownloadRequest,
) -> Result<Vec<LuaManifestRow>, String> {
    let app_id = request.download.app_id;
    let steam_path = request.download.steam_path;
    let api_key = request.api_key;
    let source = HubcapSource::from_api_key(&api_key);
    let plan = MutationPlan::prepare(
        std::path::Path::new(&steam_path),
        app_id,
        "prepare_specific_version_download",
    )?;
    if let Err(error) = validate_authenticated_download(&api_key, &steam_path, "download the Lua file") {
        crate::desk_log_error!(
            "store",
            "Specific version download failed for {}: {}",
            crate::core::logger::format_appid(app_id),
            error
        );
        return Err(error);
    }

    crate::desk_log_info!(
        "store",
        "Preparing specific version download for {} (source key: {})",
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
        let manifest_rows = LuaManifestPins::rows_from_content(&package.lua_content);
        if manifest_rows.is_empty() {
            return Err("The downloaded Lua does not contain any setManifestid entries, so it was not installed. Try another source or verify the provider returned the full Lua with manifests.".to_string());
        }

        complete_specific_package(app_id, &steam_path, &mut package, &api_key).await?;
        install_specific_package(
            app,
            app_id,
            &steam_path,
            package,
            source.display_name(),
            plan,
            SpecificInstallPolicy {
                clear_residual_download: false,
            },
        )
        .await
    }
    .await;

    match &result {
        Ok(rows) => crate::desk_log_info!(
            "store",
            "Successfully prepared specific version download for {}: {} row(s) installed",
            crate::core::logger::format_appid(app_id),
            rows.len()
        ),
        Err(error) => crate::desk_log_error!(
            "store",
            "Specific version download failed for {}: {}",
            crate::core::logger::format_appid(app_id),
            error
        ),
    }
    result
}

pub async fn prepare_ryuu_specific_version_download(
    app: &tauri::AppHandle,
    request: AuthenticatedDownloadRequest,
) -> Result<Vec<LuaManifestRow>, String> {
    let app_id = request.download.app_id;
    let steam_path = request.download.steam_path;
    let api_key = request.api_key;
    let plan = MutationPlan::prepare(
        std::path::Path::new(&steam_path),
        app_id,
        "prepare_ryuu_specific_version_download",
    )?;
    validate_authenticated_download(&api_key, &steam_path, "download the Lua file from Ryuu")?;
    crate::desk_log_info!(
        "store",
        "Preparing Ryuu specific version download for {}",
        crate::core::logger::format_appid(app_id)
    );

    let package = RyuuClient::new(api_key).download_lua_package(app_id).await?;
    let installed_rows = install_specific_package(
        app,
        app_id,
        &steam_path,
        package,
        "Ryuu",
        plan,
        SpecificInstallPolicy::default(),
    )
    .await?;
    crate::desk_log_info!(
        "store",
        "Successfully prepared Ryuu specific version download for {}: {} row(s) installed",
        crate::core::logger::format_appid(app_id),
        installed_rows.len()
    );
    Ok(installed_rows)
}

pub async fn prepare_luatools_specific_version_download(
    app: &tauri::AppHandle,
    request: LuaToolsDownloadRequest,
) -> Result<Vec<LuaManifestRow>, String> {
    let app_id = request.download.app_id;
    let steam_path = request.download.steam_path;
    let plan = MutationPlan::prepare(
        std::path::Path::new(&steam_path),
        app_id,
        "prepare_luatools_specific_version_download",
    )?;
    validate_steam_download_path(&steam_path)?;
    crate::desk_log_info!(
        "store",
        "Preparing LuaTools specific-version download for {}",
        crate::core::logger::format_appid(app_id)
    );
    let package = download_complete_luatools_package(
        app,
        app_id,
        &steam_path,
        request.game_name.as_deref(),
    )
    .await?;
    install_specific_package(
        app,
        app_id,
        &steam_path,
        package,
        "LuaTools",
        plan,
        SpecificInstallPolicy::default(),
    )
    .await
}
