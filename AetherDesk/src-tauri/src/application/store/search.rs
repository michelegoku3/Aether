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
use crate::store::genres::GenreDetector;
use crate::store::service::{StoreService, UnifiedStoreGame};
use std::collections::HashMap;

pub async fn suggest_store_games(
    app: &tauri::AppHandle,
    query: &str,
) -> Result<Vec<StoreSuggestItem>, String> {
    let settings = load_settings(app);
    let store_currency = normalize_store_currency(&settings.store_currency);
    let country = steam_country_code_for_currency(&store_currency);
    let items = crate::steam::store::SteamStore::new()
        .suggest_for_country(query.trim(), country, &store_currency)
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
        crate::desk_log_debug!(
            "store",
            "Store search cache hit state=fresh query_len={} results={}",
            query.len(),
            results.len()
        );
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
            &store_currency,
        )
        .await
    {
        Ok(results) => {
            SteamAppNameResolver::new(&cache_dir)
                .merge_names(results.iter().map(|game| (game.id, game.name.clone())));
            GameInfoCache::new(&cache_dir, info_cache_version)
                .merge_store_results_with_manifest_context(&results, hubcap_checked);
            match cache.put(&cache_key, results.clone()) {
                Ok(()) => crate::desk_log_debug!(
                    "store",
                    "Store search cache write complete query_len={} results={}",
                    query.len(),
                    results.len()
                ),
                Err(error) => crate::desk_log_warn!(
                    "store",
                    "Store search cache write failed query_len={}: {}",
                    query.len(),
                    error
                ),
            }
            crate::desk_log_info!(
                "store",
                "Store search query_len={} completed results={}",
                query.len(),
                results.len()
            );
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
    store_front_filter_override: Option<String>,
) -> Result<Vec<UnifiedStoreGame>, String> {
    let settings = load_settings(app);
    if !settings.show_store_front_games {
        return Ok(Vec::new());
    }

    let show_store_dlcs = settings.show_store_dlcs;
    let show_store_nsfw = settings.show_store_nsfw;
    let show_store_delisted = settings.show_store_delisted;
    let store_front_filter = store_front_filter_override
        .map(|v| normalize_store_front_filter(&v))
        .unwrap_or_else(|| normalize_store_front_filter(&settings.store_front_filter));
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
            &store_currency,
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

/// Trending items the Store requests on its first preload (2 × the 20-item
/// page grid). The deferred warm must use the exact page the Store reads on
/// open, or the cache key would miss and the click would wait on Steam again.
pub const STORE_FRONT_FIRST_PAGE: usize = 40;

/// Fills the trending cache for the Store's first page once the window is
/// interactive, so opening the Store reads a fresh entry instead of waiting on
/// the Steam store front. Best-effort: a failure is logged, never surfaced.
pub async fn warm_store_front(app: &tauri::AppHandle) {
    warm_store_front_with(|start, count| get_trending_store_games(app, start, count, None)).await;
}

async fn warm_store_front_with<F, Fut>(fetch: F)
where
    F: FnOnce(usize, usize) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<UnifiedStoreGame>, String>>,
{
    match fetch(0, STORE_FRONT_FIRST_PAGE).await {
        Ok(games) => crate::desk_log_debug!(
            "store",
            "Store front warm filled {} trending game(s)",
            games.len()
        ),
        Err(error) => crate::desk_log_debug!("store", "Store front warm skipped: {}", error),
    }
}

pub fn get_cached_store_search(app: &tauri::AppHandle, query: &str) -> CachedStoreSearchResponse {
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
) -> Result<HashMap<u32, String>, String> {
    let cache_dir = LocalAppPaths::data_root().join("cache");
    let app_version = app.package_info().version.to_string();
    let settings = load_settings(app);
    let info_cache_version = cache_version_with_currency(&app_version, &settings.store_currency);
    // Keyed on the DRM schema, NOT the package version: tauri.conf.json stays at
    // 1.4.3 across releases, so a version key would never invalidate stale entries.
    let results = DrmDetector::new(&cache_dir, crate::store::drm::DRM_CACHE_SCHEMA.to_string())
        .detect_many(app_ids)
        .await?;
    GameInfoCache::new(&cache_dir, info_cache_version).merge_denuvo_flags(&results);
    Ok(results)
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct CrackInfo {
    pub status: String,
}

pub async fn check_crack_status_bulk(
    app: &tauri::AppHandle,
    app_ids: Vec<u32>,
) -> Result<HashMap<u32, CrackInfo>, String> {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    const DEFAULT_SUPABASE_URL: &str = "https://lhvknkrfhehcclzlabsl.supabase.co";
    const DEFAULT_SUPABASE_ANON_KEY: &str =
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6Imxodmtua3JmaGVoY2NsemxhYnNsIiwicm9sZSI6ImFub24iLCJpYXQiOjE3Nzg4NzM0MTksImV4cCI6MjA5NDQ0OTQxOX0.B7YOW_hpn2zHxR-sfHgiNgqidpfESwJpixLrh-MevE8";
    const CACHE_TTL: u64 = 24 * 60 * 60;
    const MAX_APP_IDS: usize = 200;

    if app_ids.is_empty() {
        return Ok(HashMap::new());
    }
    if app_ids.len() > MAX_APP_IDS {
        return Err(format!(
            "Too many app ids for crack status check ({} > {})",
            app_ids.len(),
            MAX_APP_IDS
        ));
    }
    // Deduplicate while preserving order so the cache and the request payload
    // never carry the same id twice.
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let app_ids: Vec<u32> = app_ids.into_iter().filter(|id| seen.insert(*id)).collect();

    let settings = load_settings(app);
    let supabase_url = if settings.supabase_url.is_empty() {
        DEFAULT_SUPABASE_URL
    } else {
        &settings.supabase_url
    };
    let supabase_anon_key = if settings.supabase_anon_key.is_empty() {
        DEFAULT_SUPABASE_ANON_KEY
    } else {
        &settings.supabase_anon_key
    };

    let cache_dir = LocalAppPaths::data_root().join("cache");
    let _ = fs::create_dir_all(&cache_dir);
    let cache_path = cache_dir.join("crack_status_cache.json");

    let mut result: HashMap<u32, CrackInfo> = HashMap::new();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Load cache.
    let mut cache: serde_json::Value = serde_json::Value::Null;
    if let Ok(content) = fs::read_to_string(&cache_path) {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
            cache = parsed;
        }
    }
    let mut need_fetch: Vec<u32> = Vec::new();
    for id in &app_ids {
        if let Some(obj) = cache.get("entries").and_then(|v| v.as_object()) {
            if let Some(entry) = obj.get(&id.to_string()) {
                let ts = entry.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
                if now.saturating_sub(ts) <= CACHE_TTL {
                    let status = entry
                        .get("status")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    if let Some(st) = status {
                        result.insert(*id, CrackInfo { status: st });
                        continue;
                    }
                }
            }
        }
        need_fetch.push(*id);
    }

    if need_fetch.is_empty() {
        return Ok(result);
    }

    // Fetch from Supabase.
    let ids_csv = need_fetch
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let url = format!(
        "{}/rest/v1/games?select=steam_appid,status&steam_appid=in.({})",
        supabase_url, ids_csv
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    if let Ok(resp) = client
        .get(&url)
        .header("apikey", supabase_anon_key)
        .header("Authorization", format!("Bearer {}", supabase_anon_key))
        .send()
        .await
    {
        if let Ok(json) = resp.json::<serde_json::Value>().await {
            if let Some(arr) = json.as_array() {
                for item in arr {
                    let appid = item
                        .get("steam_appid")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<u32>().ok());
                    let status = item.get("status").and_then(|v| v.as_str());
                    if let (Some(id), Some(st)) = (appid, status) {
                        result.insert(
                            id,
                            CrackInfo {
                                status: st.to_string(),
                            },
                        );
                    }
                }
            }
        }
    }

    // Update cache.
    let mut entries = cache
        .get("entries")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if !entries.is_object() {
        entries = serde_json::json!({});
    }
    let obj = entries.as_object_mut().unwrap();
    for id in &need_fetch {
        if let Some(info) = result.get(id) {
            obj.insert(
                id.to_string(),
                serde_json::json!({"status": info.status, "ts": now}),
            );
        }
    }
    let stamped = serde_json::json!({"entries": entries});
    let _ = fs::write(
        &cache_path,
        serde_json::to_string(&stamped).unwrap_or_default(),
    );

    Ok(result)
}

fn percent_encode(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

pub async fn check_pc_drm_bulk(
    _app: &tauri::AppHandle,
    app_ids: Vec<u32>,
    names: Vec<String>,
) -> Result<HashMap<u32, String>, String> {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    const CACHE_TTL: u64 = 7 * 24 * 60 * 60;
    let cache_dir = LocalAppPaths::data_root().join("cache");
    let _ = fs::create_dir_all(&cache_dir);
    let cache_path = cache_dir.join("pc_drm_cache.json");

    let mut result: HashMap<u32, String> = HashMap::new();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Load cache.
    let mut cache: serde_json::Value = serde_json::Value::Null;
    if let Ok(content) = fs::read_to_string(&cache_path) {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
            cache = parsed;
        }
    }

    let mut need_fetch: Vec<(u32, String)> = Vec::new();
    for (id, name) in app_ids.iter().zip(names.iter()) {
        if let Some(obj) = cache.get("entries").and_then(|v| v.as_object()) {
            if let Some(entry) = obj.get(&id.to_string()) {
                let ts = entry.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
                if now.saturating_sub(ts) <= CACHE_TTL {
                    if let Some(drm) = entry.get("drm").and_then(|v| v.as_str()) {
                        result.insert(*id, drm.to_string());
                        continue;
                    }
                }
            }
        }
        need_fetch.push((*id, name.clone()));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent("AetherDesk/1.0")
        .build()
        .map_err(|e| e.to_string())?;

    for (id, name) in &need_fetch {
        // Search PCGamingWiki by name.
        let search_url = format!(
            "https://www.pcgamingwiki.com/w/api.php?action=query&list=search&srsearch={}&srlimit=5&format=json",
            percent_encode(name)
        );
        let title = match client.get(&search_url).send().await {
            Ok(resp) => {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    json.get("query")
                        .and_then(|q| q.get("search"))
                        .and_then(|s| s.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|o| o.get("title"))
                        .and_then(|t| t.as_str())
                        .map(String::from)
                } else {
                    None
                }
            }
            Err(_) => None,
        };

        let Some(title) = title else {
            continue;
        };

        // Fetch wikitext.
        let content_url = format!(
            "https://www.pcgamingwiki.com/w/api.php?action=query&prop=revisions&rvprop=content&titles={}&format=json",
            percent_encode(&title)
        );
        let wikitext = match client.get(&content_url).send().await {
            Ok(resp) => {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    json.get("query")
                        .and_then(|q| q.get("pages"))
                        .and_then(|p| p.as_object())
                        .and_then(|o| o.values().next())
                        .and_then(|v| v.get("revisions"))
                        .and_then(|r| r.as_array())
                        .and_then(|a| a.first())
                        .and_then(|r| r.get("*"))
                        .and_then(|c| c.as_str())
                        .map(String::from)
                } else {
                    None
                }
            }
            Err(_) => None,
        };

        let Some(wt) = wikitext else {
            continue;
        };
        // Verify steam appid present to avoid wrong game.
        if !wt.contains(&id.to_string()) {
            continue;
        }
        // Parse DRM template. "{{DRM|" is 6 chars, so +6 lands on the value;
        // +7 would silently eat its first letter ("Denuvo" -> "enuvo").
        let drm = if let Some(start) = wt.find("{{DRM|") {
            let rest = &wt[start + 6..];
            if let Some(end) = rest.find("}}") {
                let val = rest[..end].trim();
                if val.eq_ignore_ascii_case("none") || val.is_empty() {
                    "None".to_string()
                } else {
                    // Take first DRM name (before comma or pipe).
                    val.split([',', '|'])
                        .next()
                        .unwrap_or(val)
                        .trim()
                        .to_string()
                }
            } else {
                continue;
            }
        } else {
            continue;
        };

        // Sanitize: only accept clean DRM names; skip messy values.
        let lower = drm.to_lowercase();
        if lower.contains("team")
            || lower.contains("offline")
            || lower.contains("ffline")
            || lower.contains("=")
            || drm.len() > 30
        {
            continue;
        }
        result.insert(*id, drm);
    }

    // Update cache.
    let mut entries = cache
        .get("entries")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if !entries.is_object() {
        entries = serde_json::json!({});
    }
    let obj = entries.as_object_mut().unwrap();
    for (id, drm) in &result {
        // only cache newly fetched? cache all results present
        if need_fetch.iter().any(|(x, _)| x == id) {
            obj.insert(id.to_string(), serde_json::json!({"drm": drm, "ts": now}));
        }
    }
    let stamped = serde_json::json!({"entries": entries});
    let _ = fs::write(
        &cache_path,
        serde_json::to_string(&stamped).unwrap_or_default(),
    );

    Ok(result)
}

pub async fn get_genres_bulk(
    app: &tauri::AppHandle,
    app_ids: Vec<u32>,
) -> Result<HashMap<u32, Vec<String>>, String> {
    let cache_dir = LocalAppPaths::data_root().join("cache");
    let app_version = app.package_info().version.to_string();
    let results = GenreDetector::new(&cache_dir, app_version)
        .detect_many(app_ids)
        .await?;
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
    use super::{
        build_store_cache_key, build_trending_cache_key, warm_store_front_with,
        STORE_FRONT_FIRST_PAGE,
    };
    use crate::store::service::UnifiedStoreGame;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn warm_store_front_fetches_the_first_page_and_swallows_failures() {
        let requested = Arc::new(Mutex::new(None::<(usize, usize)>));
        let requested_by_fetch = Arc::clone(&requested);

        warm_store_front_with(move |start, count| {
            *requested_by_fetch.lock().unwrap() = Some((start, count));
            async move { Err::<Vec<UnifiedStoreGame>, String>("offline".to_string()) }
        })
        .await;

        assert_eq!(*requested.lock().unwrap(), Some((0, STORE_FRONT_FIRST_PAGE)));
    }

    #[test]
    fn search_cache_key_includes_every_result_shaping_setting() {
        let key = build_store_cache_key(true, "EUR", true, false, true, "portal");
        assert_eq!(
            key,
            "hubcap|currency=EUR|dlcs=true|nsfw=false|delisted=true portal"
        );
    }

    #[test]
    fn trending_cache_key_includes_page_and_filter() {
        let key = build_trending_cache_key("USD", "top_sellers", false, true, false, 12, 24);
        assert_eq!(key, "storefront=top_sellers|steam|currency=USD|dlcs=false|nsfw=true|delisted=false|start=12|count=24");
    }
}
