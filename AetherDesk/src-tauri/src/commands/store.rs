use crate::core::backup::GameBackup;
use crate::game_info::cache::GameInfoCache;
use crate::store::download::DownloadOrchestrator;
use crate::store::drm::DrmDetector;
use crate::providers::hubcap::HubcapClient;
use crate::providers::ryuu::RyuuClient;
use crate::core::paths::LocalAppPaths;
use crate::manifest::pins::{LuaManifestPins, LuaManifestRow};
use crate::manifest::package::ManifestPackage;
use crate::core::settings::{cache_version_with_currency, normalize_store_currency, normalize_store_front_filter, steam_country_code_for_currency, SettingsManager};
use crate::steam::app_names::SteamAppNameResolver;
use crate::steam::compat::SteamCompat;
use crate::store::cache::StoreSearchCache;
use crate::store::service::{StoreService, UnifiedStoreGame};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedStoreSearchResponse {
    pub results: Vec<UnifiedStoreGame>,
    /// `fresh` = 24h cache hit; `stale` = immediate 14-day fallback that the
    /// frontend should refresh in the background; `miss` = no usable cache.
    pub cache_state: String,
}

#[tauri::command]
pub async fn search_store(
    app: tauri::AppHandle,
    query: String,
) -> Result<Vec<UnifiedStoreGame>, String> {
    let settings = SettingsManager::new(&app).load();
    let show_store_dlcs = settings.show_store_dlcs;
    let show_store_nsfw = settings.show_store_nsfw;
    let show_store_delisted = settings.show_store_delisted;
    let store_currency = normalize_store_currency(&settings.store_currency);
    let steam_country_code = steam_country_code_for_currency(&store_currency);
    let app_version = app.package_info().version.to_string();
    let info_cache_version = cache_version_with_currency(&app_version, &store_currency);
    let hubcap_client = (!settings.hubcap_api_key.trim().is_empty())
        .then(|| HubcapClient::new(settings.hubcap_api_key));
    let hubcap_checked = hubcap_client.is_some();

    let cache = StoreSearchCache::new(
        LocalAppPaths::data_root().join("cache"),
        app_version.clone(),
    );
    // The filter flags are part of the cache key: toggling any setting must
    // not replay 24h-stale results built under other flag values.
    let cache_key = build_store_cache_key(
        hubcap_client.is_some(),
        &store_currency,
        show_store_dlcs,
        show_store_nsfw,
        show_store_delisted,
        &query,
    );

    if let Some(results) = cache.get_fresh(&cache_key) {
        GameInfoCache::new(
            LocalAppPaths::data_root().join("cache"),
            info_cache_version.clone(),
        )
        .merge_store_results_with_manifest_context(&results, hubcap_checked);
        return Ok(results);
    }

    match StoreService::new()
        .search_store(&query, hubcap_client, show_store_dlcs, show_store_nsfw, show_store_delisted, steam_country_code)
        .await
    {
        Ok(results) => {
            let cache_dir = LocalAppPaths::data_root().join("cache");
            SteamAppNameResolver::new(cache_dir.clone())
                .merge_names(results.iter().map(|game| (game.id, game.name.clone())));
            GameInfoCache::new(cache_dir, info_cache_version.clone())
                .merge_store_results_with_manifest_context(&results, hubcap_checked);
            let _ = cache.put(&cache_key, results.clone());
            Ok(results)
        }
        Err(error) => {
            if let Some(results) = cache.get_any(&cache_key) {
                GameInfoCache::new(
                    LocalAppPaths::data_root().join("cache"),
                    info_cache_version.clone(),
                )
                .merge_store_results_with_manifest_context(&results, hubcap_checked);
                Ok(results)
            } else {
                Err(error)
            }
        }
    }
}

#[tauri::command]
pub async fn get_trending_store_games(
    app: tauri::AppHandle,
    start: usize,
    count: usize,
) -> Result<Vec<UnifiedStoreGame>, String> {
    let settings = SettingsManager::new(&app).load();
    if !settings.show_store_front_games {
        return Ok(Vec::new());
    }

    let show_store_dlcs = settings.show_store_dlcs;
    let show_store_nsfw = settings.show_store_nsfw;
    let show_store_delisted = settings.show_store_delisted;
    let store_front_filter = normalize_store_front_filter(&settings.store_front_filter);
    let store_currency = normalize_store_currency(&settings.store_currency);
    let steam_country_code = steam_country_code_for_currency(&store_currency);
    let app_version = app.package_info().version.to_string();
    let info_cache_version = cache_version_with_currency(&app_version, &store_currency);
    let hubcap_client = (!settings.hubcap_api_key.trim().is_empty())
        .then(|| HubcapClient::new(settings.hubcap_api_key));
    let hubcap_checked = hubcap_client.is_some();

    let cache = StoreSearchCache::new(
        LocalAppPaths::data_root().join("cache"),
        app_version,
    );
    let cache_key = build_trending_cache_key(
        hubcap_checked,
        &store_currency,
        &store_front_filter,
        show_store_dlcs,
        show_store_nsfw,
        show_store_delisted,
        start,
        count,
    );

    if let Some(results) = cache.get_fresh(&cache_key) {
        GameInfoCache::new(
            LocalAppPaths::data_root().join("cache"),
            info_cache_version.clone(),
        )
        .merge_store_results_with_manifest_context(&results, hubcap_checked);
        return Ok(results);
    }

    match StoreService::new()
        .trending_store(
            &store_front_filter,
            start,
            count,
            hubcap_client,
            show_store_dlcs,
            show_store_nsfw,
            show_store_delisted,
            steam_country_code,
        )
        .await
    {
        Ok(results) => {
            let cache_dir = LocalAppPaths::data_root().join("cache");
            SteamAppNameResolver::new(cache_dir.clone())
                .merge_names(results.iter().map(|game| (game.id, game.name.clone())));
            GameInfoCache::new(cache_dir, info_cache_version)
                .merge_store_results_with_manifest_context(&results, hubcap_checked);
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
pub fn get_cached_store_search(
    app: tauri::AppHandle,
    query: String,
) -> Result<CachedStoreSearchResponse, String> {
    if query.trim().is_empty() {
        return Ok(CachedStoreSearchResponse {
            results: Vec::new(),
            cache_state: "miss".to_string(),
        });
    }

    let settings = SettingsManager::new(&app).load();
    let store_currency = normalize_store_currency(&settings.store_currency);
    let hubcap_enabled = !settings.hubcap_api_key.trim().is_empty();
    let cache = StoreSearchCache::new(
        LocalAppPaths::data_root().join("cache"),
        app.package_info().version.to_string(),
    );
    let cache_key = build_store_cache_key(
        hubcap_enabled,
        &store_currency,
        settings.show_store_dlcs,
        settings.show_store_nsfw,
        settings.show_store_delisted,
        &query,
    );

    if let Some(results) = cache.get_fresh(&cache_key) {
        return Ok(CachedStoreSearchResponse {
            results,
            cache_state: "fresh".to_string(),
        });
    }

    if let Some(results) = cache.get_stale(&cache_key) {
        return Ok(CachedStoreSearchResponse {
            results,
            cache_state: "stale".to_string(),
        });
    }

    Ok(CachedStoreSearchResponse {
        results: Vec::new(),
        cache_state: "miss".to_string(),
    })
}

#[tauri::command]
pub async fn check_denuvo_bulk(
    app: tauri::AppHandle,
    app_ids: Vec<u32>,
) -> Result<HashMap<u32, bool>, String> {
    let cache_dir = LocalAppPaths::data_root().join("cache");
    let app_version = app.package_info().version.to_string();
    let settings = SettingsManager::new(&app).load();
    let info_cache_version = cache_version_with_currency(&app_version, &settings.store_currency);
    let results = DrmDetector::new(cache_dir.clone(), app_version)
        .detect_many(app_ids)
        .await?;
    GameInfoCache::new(cache_dir, info_cache_version).merge_denuvo_flags(&results);
    Ok(results)
}

#[tauri::command]
pub async fn trigger_hubcap_download(
    app: tauri::AppHandle,
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

    apply_default_update_policy(&app, app_id, &steam_path)?;
    let installed_lua = steam.read_lua_config(app_id).unwrap_or(package.lua_content.clone());

    // Centralized Lua/manifest backup (AetherData/backup/<app_id>/lua).
    GameBackup::for_app(app_id)?
        .backup_lua_artifacts(app_id, &installed_lua, &package.manifest_files)?;
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

#[tauri::command]
pub async fn trigger_ryuu_download(
    app: tauri::AppHandle,
    app_id: u32,
    api_key: String,
    steam_path: String,
) -> Result<String, String> {
    validate_download_inputs(&api_key, &steam_path, "call Ryuu")?;

    let steam = SteamCompat::new(steam_path.clone());
    let client = RyuuClient::new(api_key);
    let package = client.download_lua_package(app_id).await?;
    steam.install_lua_config(app_id, &package.lua_content)?;
    steam.install_manifest_files(&package.manifest_files)?;
    apply_default_update_policy(&app, app_id, &steam_path)?;
    let installed_lua = steam.read_lua_config(app_id).unwrap_or(package.lua_content.clone());

    GameBackup::for_app(app_id)?
        .backup_lua_artifacts(app_id, &installed_lua, &package.manifest_files)?;
    let manifest_count = package.manifest_files.len();

    Ok(format!(
        "Successfully completed Ryuu download for App ID {}. Lua installed, {} manifest file(s) preloaded into Steam depotcache.",
        app_id, manifest_count
    ))
}

#[tauri::command]
pub async fn prepare_ryuu_specific_version_download(
    _app: tauri::AppHandle,
    app_id: u32,
    api_key: String,
    steam_path: String,
) -> Result<Vec<LuaManifestRow>, String> {
    validate_download_inputs(&api_key, &steam_path, "download the Lua file from Ryuu")?;

    let client = RyuuClient::new(api_key);
    let package = client.download_lua_package(app_id).await?;
    let lua_content = package.lua_content;
    let manifest_rows = LuaManifestPins::rows_from_content(&lua_content);

    if manifest_rows.is_empty() {
        return Err("The downloaded Lua from Ryuu does not contain any setManifestid entries, so it was not installed. Try another source or verify the provider returned the full Lua with manifests.".to_string());
    }

    let steam = SteamCompat::new(steam_path.clone());
    steam.install_lua_config(app_id, &lua_content)?;
    steam.install_manifest_files(&package.manifest_files)?;
    GameBackup::for_app(app_id)?
        .backup_lua_artifacts(app_id, &lua_content, &package.manifest_files)?;

    let installed_rows = LuaManifestPins::new(steam_path, app_id).rows_from_file()?;
    if installed_rows.len() != manifest_rows.len() {
        return Err(format!(
            "Lua install verification failed: downloaded file had {} setManifestid entries, installed file has {}.",
            manifest_rows.len(),
            installed_rows.len()
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

fn apply_default_update_policy(
    app: &tauri::AppHandle,
    app_id: u32,
    steam_path: &str,
) -> Result<(), String> {
    let settings = SettingsManager::new(app).load();
    if settings.download_games_with_updates_on {
        LuaManifestPins::new(steam_path.to_string(), app_id)
            .set_updates_enabled(true)
            .map(|_| ())?;
    }
    Ok(())
}

fn build_store_cache_key(
    hubcap_enabled: bool,
    store_currency: &str,
    show_store_dlcs: bool,
    show_store_nsfw: bool,
    show_store_delisted: bool,
    query: &str,
) -> String {
    format!(
        "{}|currency={}|dlcs={}|nsfw={}|delisted={} {}",
        if hubcap_enabled { "hubcap" } else { "steam" },
        store_currency,
        show_store_dlcs,
        show_store_nsfw,
        show_store_delisted,
        query
    )
}

fn build_trending_cache_key(
    hubcap_enabled: bool,
    store_currency: &str,
    store_front_filter: &str,
    show_store_dlcs: bool,
    show_store_nsfw: bool,
    show_store_delisted: bool,
    start: usize,
    count: usize,
) -> String {
    format!(
        "storefront={}|{}|currency={}|dlcs={}|nsfw={}|delisted={}|start={}|count={}",
        store_front_filter,
        if hubcap_enabled { "hubcap" } else { "steam" },
        store_currency,
        show_store_dlcs,
        show_store_nsfw,
        show_store_delisted,
        start,
        count,
    )
}
