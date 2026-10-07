//! Tauri IPC adapter for store operations.
//!
//! This module deliberately contains no provider, cache, manifest or Steam
//! workflow logic. It resolves request-scoped backend context, constructs
//! typed application requests and preserves the existing IPC surface.

use super::command_steam_path;
use crate::application::store;
use crate::manifest::pins::LuaManifestRow;
use crate::store::service::UnifiedStoreGame;
use std::collections::HashMap;

pub use crate::application::store::{CachedStoreSearchResponse, StoreSuggestItem};

#[tauri::command]
pub async fn suggest_store_games(
    app: tauri::AppHandle,
    query: String,
) -> Result<Vec<StoreSuggestItem>, String> {
    store::suggest_store_games(&app, &query).await
}

#[tauri::command]
pub async fn search_store(
    app: tauri::AppHandle,
    query: String,
) -> Result<Vec<UnifiedStoreGame>, String> {
    store::search_store(&app, &query).await
}

#[tauri::command]
pub async fn get_trending_store_games(
    app: tauri::AppHandle,
    start: usize,
    count: usize,
) -> Result<Vec<UnifiedStoreGame>, String> {
    store::get_trending_store_games(&app, start, count).await
}

#[tauri::command]
pub fn get_cached_store_search(
    app: tauri::AppHandle,
    query: String,
) -> Result<CachedStoreSearchResponse, String> {
    Ok(store::get_cached_store_search(&app, &query))
}

#[tauri::command]
pub async fn check_denuvo_bulk(
    app: tauri::AppHandle,
    app_ids: Vec<u32>,
) -> Result<HashMap<u32, bool>, String> {
    store::check_denuvo_bulk(&app, app_ids).await
}

#[tauri::command]
pub async fn trigger_hubcap_download(
    app: tauri::AppHandle,
    app_id: u32,
    api_key: String,
) -> Result<String, String> {
    let request = authenticated_request(&app, app_id, api_key)?;
    store::trigger_hubcap_download(&app, request).await
}

#[tauri::command]
pub async fn prepare_specific_version_download(
    app: tauri::AppHandle,
    app_id: u32,
    api_key: String,
) -> Result<Vec<LuaManifestRow>, String> {
    let request = authenticated_request(&app, app_id, api_key)?;
    store::prepare_specific_version_download(&app, request).await
}

#[tauri::command]
pub async fn trigger_ryuu_download(
    app: tauri::AppHandle,
    app_id: u32,
    api_key: String,
) -> Result<String, String> {
    let request = authenticated_request(&app, app_id, api_key)?;
    store::trigger_ryuu_download(&app, request).await
}

#[tauri::command]
pub async fn prepare_ryuu_specific_version_download(
    app: tauri::AppHandle,
    app_id: u32,
    api_key: String,
) -> Result<Vec<LuaManifestRow>, String> {
    let request = authenticated_request(&app, app_id, api_key)?;
    store::prepare_ryuu_specific_version_download(&app, request).await
}

#[tauri::command]
pub async fn trigger_luatools_download(
    app: tauri::AppHandle,
    app_id: u32,
    game_name: Option<String>,
) -> Result<String, String> {
    let request = luatools_request(&app, app_id, game_name)?;
    store::trigger_luatools_download(&app, request).await
}

#[tauri::command]
pub async fn prepare_luatools_specific_version_download(
    app: tauri::AppHandle,
    app_id: u32,
    game_name: Option<String>,
) -> Result<Vec<LuaManifestRow>, String> {
    let request = luatools_request(&app, app_id, game_name)?;
    store::prepare_luatools_specific_version_download(&app, request).await
}

fn authenticated_request(
    app: &tauri::AppHandle,
    app_id: u32,
    api_key: String,
) -> Result<store::AuthenticatedDownloadRequest, String> {
    Ok(store::AuthenticatedDownloadRequest {
        download: download_request(app, app_id)?,
        api_key,
    })
}

fn luatools_request(
    app: &tauri::AppHandle,
    app_id: u32,
    game_name: Option<String>,
) -> Result<store::LuaToolsDownloadRequest, String> {
    Ok(store::LuaToolsDownloadRequest {
        download: download_request(app, app_id)?,
        game_name,
    })
}

fn download_request(
    app: &tauri::AppHandle,
    app_id: u32,
) -> Result<store::DownloadRequest, String> {
    Ok(store::DownloadRequest {
        app_id,
        steam_path: command_steam_path(app)?,
    })
}
