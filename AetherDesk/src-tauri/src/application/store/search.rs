//! Store discovery, cache fallback and DRM enrichment use cases.

use super::model::{CachedStoreSearchResponse, StoreSuggestItem};
use crate::core::paths::LocalAppPaths;
use crate::core::settings::{
    cache_version_with_currency, load_settings, normalize_store_currency,
    normalize_store_front_filter, steam_country_code_for_currency,
};
use crate::game_info::cache::GameInfoCache;
use crate::providers::hubcap::HubcapClient;
use crate::steam::app_names::SteamAppNameResolver;
use crate::store::cache::StoreSearchCache;
use crate::store::drm::DrmDetector;
use crate::store::service::{StoreService, UnifiedStoreGame};
use std::collections::HashMap;

pub async fn suggest_store_games(
    app: &tauri::AppHandle,
    query: &str,
) -> Result<Vec<StoreSuggestItem>, String> {
    let settings = load_settings(app);
    let country = steam_country_code_for_currency(&settings.store_currency);
    let items = crate::steam::store::SteamStore::new()
        .suggest_for_country(query.trim(), country)
        .await?;
    Ok(items
        .into_iter()
        .map(|item| StoreSuggestItem {
            id: item.id,
            name: item.name,
            app_id: item.id.to_string(),
        })
        .collect())
}

pub async fn search_store(
    app: &tauri::AppHandle,
    query: &str,
) -> Result<Vec<UnifiedStoreGame>, String> {
    let settings = load_settings(app);
    let show_store_dlcs = settings.show_store_dlcs;
    let show_store_nsfw = settings.show_store_nsfw;
    let show_store_delisted = settings.show_store_delisted;
    let store_currency = normalize_store_currency(&settings.store_currency);
    let steam_country_code = steam_country_code_for_currency(&store_currency);
    let app_version = app.package_info().version.to_string();
    let info_cache_version = cache_version_with_currency(&app_version, &store_currency);
    let hubcap_client = (!settings.hubcap_api_key.trim().is_empty())
        .then(|| HubcapClient::new(&settings.hubcap_api_key));
    let hubcap_checked = hubcap_client.is_some();

    crate::desk_log_info!("store", "Searching store for query='{}' (currency={}, dlcs={}, nsfw={}, delisted={}, hubcap_key_active={})",
        query, store_currency, show_store_dlcs, show_store_nsfw, show_store_delisted, hubcap_checked);

    let cache_dir = LocalAppPaths::data_root().join("cache");
    let cache = StoreSearchCache::new(&cache_dir, &app_version);
    let cache_key = build_store_cache_key(
        hubcap_client.is_some(),
        &store_currency,
        show_store_dlcs,
        show_store_nsfw,
        show_store_delisted,
        query,
    );

    if let Some(results) = cache.get_fresh(&cache_key) {
        crate::desk_log_debug!("store", "Store search cache hit state=fresh query_len={} results={}", query.len(), results.len());
        GameInfoCache::new(&cache_dir, info_cache_version.clone())
            .merge_store_results_with_manifest_context(&results, hubcap_checked);
        return Ok(results);
    }

    match StoreService::new()
        .search_store(
            query,
            hubcap_client,
            show_store_dlcs,
            show_store_nsfw,
            show_store_delisted,
            steam_country_code,
        )
        .await
    {
        Ok(results) => {
            SteamAppNameResolver::new(&cache_dir)
                .merge_names(results.iter().map(|game| (game.id, game.name.clone())));
            GameInfoCache::new(&cache_dir, info_cache_version)
                .merge_store_results_with_manifest_context(&results, hubcap_checked);
            match cache.put(&cache_key, results.clone()) {
                Ok(()) => crate::desk_log_debug!("store", "Store search cache write complete query_len={} results={}", query.len(), results.len()),
                Err(error) => crate::desk_log_warn!("store", "Store search cache write failed query_len={}: {}", query.len(), error),
            }
            crate::desk_log_info!("store", "Store search query_len={} completed results={}", query.len(), results.len());
            Ok(results)
        }
        Err(error) => {
            if let Some(results) = cache.get_any(&cache_key) {
                GameInfoCache::new(&cache_dir, info_cache_version)
                    .merge_store_results_with_manifest_context(&results, hubcap_checked);
                crate::desk_log_warn!("store", "Store search query='{}' network error ({}); served {} fallback cached result(s)", query, error, results.len());
                Ok(results)
            } else {
                crate::desk_log_error!("store", "Store search query='{}' failed: {}", query, error);
                Err(error)
            }
        }
    }
}

pub async fn get_trending_store_games(
    app: &tauri::AppHandle,
    start: usize,
    count: usize,
) -> Result<Vec<UnifiedStoreGame>, String> {
    let settings = load_settings(app);
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
    let cache_dir = LocalAppPaths::data_root().join("cache");
    let cache = StoreSearchCache::new(&cache_dir, &app_version);
    let cache_key = build_trending_cache_key(
        &store_currency,
        &store_front_filter,
        show_store_dlcs,
        show_store_nsfw,
        show_store_delisted,
        start,
        count,
    );

    if let Some(results) = cache.get_fresh_for(&cache_key, 24 * 60 * 60) {
        GameInfoCache::new(&cache_dir, info_cache_version.clone())
            .merge_store_results_with_manifest_context(&results, false);
        return Ok(results);
    }

    match StoreService::new()
        .trending_store(
            &store_front_filter,
            start,
            count,
            show_store_dlcs,
            show_store_nsfw,
            show_store_delisted,
            steam_country_code,
        )
        .await
    {
        Ok(results) => {
            SteamAppNameResolver::new(&cache_dir)
                .merge_names(results.iter().map(|game| (game.id, game.name.clone())));
            GameInfoCache::new(&cache_dir, info_cache_version)
                .merge_store_results_with_manifest_context(&results, false);
            let _ = cache.put(&cache_key, results.clone());
            Ok(results)
        }
        Err(error) => cache.get_any(&cache_key).ok_or(error),
    }
}

pub fn get_cached_store_search(
    app: &tauri::AppHandle,
    query: &str,
) -> CachedStoreSearchResponse {
    if query.trim().is_empty() {
        return CachedStoreSearchResponse {
            results: Vec::new(),
            cache_state: "miss".to_string(),
        };
    }

    let settings = load_settings(app);
    let store_currency = normalize_store_currency(&settings.store_currency);
    let hubcap_enabled = !settings.hubcap_api_key.trim().is_empty();
    let cache = StoreSearchCache::new(
        &LocalAppPaths::data_root().join("cache"),
        &app.package_info().version.to_string(),
    );
    let cache_key = build_store_cache_key(
        hubcap_enabled,
        &store_currency,
        settings.show_store_dlcs,
        settings.show_store_nsfw,
        settings.show_store_delisted,
        query,
    );

    let (results, cache_state) = if let Some(results) = cache.get_fresh(&cache_key) {
        (results, "fresh")
    } else if let Some(results) = cache.get_stale(&cache_key) {
        (results, "stale")
    } else {
        (Vec::new(), "miss")
    };

    CachedStoreSearchResponse {
        results,
        cache_state: cache_state.to_string(),
    }
}

pub async fn check_denuvo_bulk(
    app: &tauri::AppHandle,
    app_ids: Vec<u32>,
) -> Result<HashMap<u32, bool>, String> {
    let cache_dir = LocalAppPaths::data_root().join("cache");
    let app_version = app.package_info().version.to_string();
    let settings = load_settings(app);
    let info_cache_version = cache_version_with_currency(&app_version, &settings.store_currency);
    let results = DrmDetector::new(&cache_dir, app_version)
        .detect_many(app_ids)
        .await?;
    GameInfoCache::new(&cache_dir, info_cache_version).merge_denuvo_flags(&results);
    Ok(results)
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
    store_currency: &str,
    store_front_filter: &str,
    show_store_dlcs: bool,
    show_store_nsfw: bool,
    show_store_delisted: bool,
    start: usize,
    count: usize,
) -> String {
    format!(
        "storefront={}|steam|currency={}|dlcs={}|nsfw={}|delisted={}|start={}|count={}",
        store_front_filter,
        store_currency,
        show_store_dlcs,
        show_store_nsfw,
        show_store_delisted,
        start,
        count,
    )
}

#[cfg(test)]
mod tests {
    use super::{build_store_cache_key, build_trending_cache_key};

    #[test]
    fn search_cache_key_includes_every_result_shaping_setting() {
        let key = build_store_cache_key(true, "EUR", true, false, true, "portal");
        assert_eq!(key, "hubcap|currency=EUR|dlcs=true|nsfw=false|delisted=true portal");
    }

    #[test]
    fn trending_cache_key_includes_page_and_filter() {
        let key = build_trending_cache_key("USD", "top_sellers", false, true, false, 12, 24);
        assert_eq!(key, "storefront=top_sellers|steam|currency=USD|dlcs=false|nsfw=true|delisted=false|start=12|count=24");
    }
}
