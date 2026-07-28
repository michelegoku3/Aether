use crate::app_storage::AppStorage;
use crate::download_orchestrator::DownloadOrchestrator;
use crate::drm_detector::DrmDetector;
use crate::hubcap_client::HubcapClient;
use crate::local_app_paths::LocalAppPaths;
use crate::lua_manifest_pins::{LuaManifestPins, LuaManifestRow};
use crate::settings::SettingsManager;
use crate::steam_app_names::SteamAppNameResolver;
use crate::steam_compat::SteamCompat;
use crate::store_search_cache::StoreSearchCache;
use crate::store_service::{StoreService, UnifiedStoreGame};
use std::collections::HashMap;

#[tauri::command]
pub async fn search_store(
    app: tauri::AppHandle,
    query: String,
) -> Result<Vec<UnifiedStoreGame>, String> {
    let settings = SettingsManager::new(&app).load();
    let hubcap_client = (!settings.hubcap_api_key.trim().is_empty())
        .then(|| HubcapClient::new(settings.hubcap_api_key));

    let cache = StoreSearchCache::new(LocalAppPaths::data_root().join("cache"));
    let cache_key = if hubcap_client.is_some() {
        format!("hubcap {}", query)
    } else {
        format!("steam {}", query)
    };

    if let Some(results) = cache.get_fresh(&cache_key) {
        return Ok(results);
    }

    match StoreService::new()
        .search_store(&query, hubcap_client)
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
    DrmDetector::new().detect_many(app_ids).await
}

#[tauri::command]
pub async fn trigger_hubcap_download(
    app: tauri::AppHandle,
    app_id: u32,
    api_key: String,
    steam_path: String,
) -> Result<String, String> {
    validate_download_inputs(&api_key, &steam_path, "call Hubcap Manifest")?;

    let client = HubcapClient::new(api_key);
    let steam = SteamCompat::new(steam_path.clone());
    let result = DownloadOrchestrator::new(client, steam)
        .execute_hubcap_download(app_id)
        .await?;

    if let Ok(lua_content) = SteamCompat::new(steam_path).read_lua_config(app_id) {
        let _ = AppStorage::new(&app).backup_lua(app_id, &lua_content);
    }

    Ok(format!(
        "Successfully completed download for App ID {}. Lua installed, {} manifest file(s) preloaded into Steam depotcache.",
        app_id, result.manifest_count
    ))
}

#[tauri::command]
pub async fn prepare_specific_version_download(
    app: tauri::AppHandle,
    app_id: u32,
    api_key: String,
    steam_path: String,
) -> Result<Vec<LuaManifestRow>, String> {
    validate_download_inputs(&api_key, &steam_path, "download the Lua file")?;

    let package = HubcapClient::new(api_key)
        .download_lua_package(app_id)
        .await?;
    let lua_content = package.lua_content;
    let manifest_rows = LuaManifestPins::rows_from_content(&lua_content);

    if manifest_rows.is_empty() {
        return Err("The downloaded Lua does not contain any setManifestid entries, so it was not installed. Try another source or verify the provider returned the full Lua with manifests.".to_string());
    }

    let steam = SteamCompat::new(steam_path.clone());
    steam.install_lua_config(app_id, &lua_content)?;
    steam.install_manifest_files(&package.manifest_files)?;
    let _ = AppStorage::new(&app).backup_lua(app_id, &lua_content);

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
