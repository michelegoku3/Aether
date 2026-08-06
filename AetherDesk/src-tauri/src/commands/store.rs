use crate::core::backup::GameBackup;
use crate::store::download::DownloadOrchestrator;
use crate::store::drm::DrmDetector;
use crate::providers::hubcap::HubcapClient;
use crate::core::paths::LocalAppPaths;
use crate::manifest::pins::{LuaManifestPins, LuaManifestRow};
use crate::manifest::package::ManifestPackage;
use crate::core::settings::SettingsManager;
use crate::steam::app_names::SteamAppNameResolver;
use crate::steam::compat::SteamCompat;
use crate::store::cache::StoreSearchCache;
use crate::store::service::{StoreService, UnifiedStoreGame};
use std::collections::HashMap;

#[tauri::command]
pub async fn search_store(
    app: tauri::AppHandle,
    query: String,
) -> Result<Vec<UnifiedStoreGame>, String> {
    let settings = SettingsManager::new(&app).load();
    let show_store_dlcs = settings.show_store_dlcs;
    let show_store_nsfw = settings.show_store_nsfw;
    let show_store_delisted = settings.show_store_delisted;
    let hubcap_client = (!settings.hubcap_api_key.trim().is_empty())
        .then(|| HubcapClient::new(settings.hubcap_api_key));

    let cache = StoreSearchCache::new(LocalAppPaths::data_root().join("cache"));
    // The filter flags are part of the cache key: toggling any setting must
    // not replay 24h-stale results built under other flag values.
    let cache_key = format!(
        "{}|dlcs={}|nsfw={}|delisted={} {}",
        if hubcap_client.is_some() { "hubcap" } else { "steam" },
        show_store_dlcs,
        show_store_nsfw,
        show_store_delisted,
        query
    );

    if let Some(results) = cache.get_fresh(&cache_key) {
        return Ok(results);
    }

    match StoreService::new()
        .search_store(&query, hubcap_client, show_store_dlcs, show_store_nsfw, show_store_delisted)
        .await
    {
        Ok(results) => {
            let cache_dir = LocalAppPaths::data_root().join("cache");
            SteamAppNameResolver::new(cache_dir)
                .merge_names(results.iter().map(|game| (game.id, game.name.clone())));
            let _ = cache.put(&cache_key, results.clone());
            Ok(results)
        }
        Err(error) => {
            if let Some(results) = cache.get_any(&cache_key) {
                Ok(results)
            } else {
                Err(error)
            }
        }
    }
}

#[tauri::command]
pub async fn check_denuvo_bulk(app_ids: Vec<u32>) -> Result<HashMap<u32, bool>, String> {
    let cache_dir = LocalAppPaths::data_root().join("cache");
    DrmDetector::new(cache_dir).detect_many(app_ids).await
}

#[tauri::command]
pub async fn trigger_hubcap_download(
    _app: tauri::AppHandle,
    app_id: u32,
    api_key: String,
    steam_path: String,
) -> Result<String, String> {
    validate_download_inputs(&api_key, &steam_path, "call Hubcap Manifest")?;

    let steam = SteamCompat::new(steam_path.clone());
    let package = if api_key == "oureveryday_public" {
        let oe_client = crate::providers::oureveryday::OureverydayClient::new();
        let package = oe_client.download_lua_package(app_id).await?;
        steam.install_lua_config(app_id, &package.lua_content)?;
        steam.install_manifest_files(&package.manifest_files)?;
        package
    } else {
        let client = HubcapClient::new(api_key);
        let result = DownloadOrchestrator::new(client, steam.clone())
            .execute_hubcap_download(app_id)
            .await?;
        // Reconstruct the installed package for the central backup.
        ManifestPackage {
            lua_content: steam.read_lua_config(app_id)?,
            manifest_files: result.manifest_files,
        }
    };

    // Centralized Lua/manifest backup (AetherData/backup/<app_id>/lua).
    GameBackup::for_app(app_id)?
        .backup_lua_artifacts(app_id, &package.lua_content, &package.manifest_files)?;
    let manifest_count = package.manifest_files.len();

    Ok(format!(
        "Successfully completed download for App ID {}. Lua installed, {} manifest file(s) preloaded into Steam depotcache.",
        app_id, manifest_count
    ))
}

#[tauri::command]
pub async fn prepare_specific_version_download(
    _app: tauri::AppHandle,
    app_id: u32,
    api_key: String,
    steam_path: String,
) -> Result<Vec<LuaManifestRow>, String> {
    validate_download_inputs(&api_key, &steam_path, "download the Lua file")?;

    let package = if api_key == "oureveryday_public" {
        let oe_client = crate::providers::oureveryday::OureverydayClient::new();
        oe_client.download_lua_package(app_id).await?
    } else {
        HubcapClient::new(api_key)
            .download_lua_package(app_id)
            .await?
    };
    let lua_content = package.lua_content;
    let manifest_rows = LuaManifestPins::rows_from_content(&lua_content);

    if manifest_rows.is_empty() {
        return Err("The downloaded Lua does not contain any setManifestid entries, so it was not installed. Try another source or verify the provider returned the full Lua with manifests.".to_string());
    }

    let steam = SteamCompat::new(steam_path.clone());
    steam.install_lua_config(app_id, &lua_content)?;
    steam.install_manifest_files(&package.manifest_files)?;
    // Centralized Lua/manifest backup (AetherData/backup/<app_id>/lua).
    GameBackup::for_app(app_id)?
        .backup_lua_artifacts(app_id, &lua_content, &package.manifest_files)?;

    let installed_rows = LuaManifestPins::new(steam_path, app_id).rows_from_file()?;
    if installed_rows.len() != manifest_rows.len() {
        return Err(format!(
            "Lua install verification failed: downloaded file had {} setManifestid entries, installed file has {}.",
            manifest_rows.len(), installed_rows.len()
        ));
    }

    Ok(installed_rows)
}

fn validate_download_inputs(
    api_key: &str,
    steam_path: &str,
    api_action: &str,
) -> Result<(), String> {
    if api_key.trim().is_empty() {
        return Err(format!("API Key is required to {}", api_action));
    }
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }
    Ok(())
}
