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
            if results.is_empty() {
                crate::desk_log_debug!(
                    "store",
                    "Skipping store search cache write for empty result set query_len={}",
                    query.len()
                );
            } else {
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
            if !results.is_empty() {
                let _ = cache.put(&cache_key, results.clone());
            }
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

const CRACK_CACHE_TTL: u64 = 24 * 60 * 60;

fn split_crack_cache_hits(
    entries: &serde_json::Map<String, serde_json::Value>,
    app_ids: &[u32],
    now: u64,
) -> (HashMap<u32, CrackInfo>, Vec<u32>) {
    let mut hits = HashMap::new();
    let mut need_fetch = Vec::new();
    for id in app_ids {
        if let Some(entry) = entries.get(&id.to_string()) {
            let ts = entry.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
            if let Some(status) = entry.get("status").and_then(|v| v.as_str()) {
                if now.saturating_sub(ts) <= CRACK_CACHE_TTL {
                    if !status.is_empty() {
                        hits.insert(
                            *id,
                            CrackInfo {
                                status: status.to_string(),
                            },
                        );
                    }
                    continue;
                }
            }
        }
        need_fetch.push(*id);
    }
    (hits, need_fetch)
}

fn parse_crack_status_rows(arr: &[serde_json::Value]) -> HashMap<u32, CrackInfo> {
    let mut parsed = HashMap::new();
    for item in arr {
        let appid = item.get("steam_appid").and_then(|v| {
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .or_else(|| v.as_str().and_then(|s| s.parse::<u32>().ok()))
        });
        let status = item.get("status").and_then(|v| v.as_str());
        if let (Some(id), Some(st)) = (appid, status) {
            if !st.trim().is_empty() {
                parsed.insert(
                    id,
                    CrackInfo {
                        status: st.to_string(),
                    },
                );
            }
        }
    }
    parsed
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

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut entries = load_pcgw_cache_entries(&cache_path);
    let (mut result, need_fetch) = split_crack_cache_hits(&entries, &app_ids, now);

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
    let client = crate::providers::http::shared_client(15);
    let mut fetched_rows: Option<HashMap<u32, CrackInfo>> = None;
    if let Ok(resp) = client
        .get(&url)
        .header("apikey", supabase_anon_key)
        .header("Authorization", format!("Bearer {}", supabase_anon_key))
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(arr) = json.as_array() {
                    fetched_rows = Some(parse_crack_status_rows(arr));
                }
            }
        }
    }

    // Only update cache when Supabase actually returned a valid array response;
    // transient HTTP/network failures leave the cache untouched.
    if let Some(rows) = fetched_rows {
        for id in &need_fetch {
            if let Some(info) = rows.get(id) {
                entries.insert(
                    id.to_string(),
                    serde_json::json!({"status": info.status, "ts": now}),
                );
                result.insert(*id, info.clone());
            } else {
                entries.insert(id.to_string(), serde_json::json!({"status": "", "ts": now}));
            }
        }
        let _ = save_pcgw_cache_entries(&cache_path, &entries);
    }

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

const PCGW_CONCURRENCY_LIMIT: usize = 3;
const PCGW_POSITIVE_CACHE_TTL: u64 = 7 * 24 * 60 * 60;
const PCGW_NEGATIVE_CACHE_TTL: u64 = 24 * 60 * 60;

static PCGW_CLIENT: once_cell::sync::Lazy<reqwest::Client> = once_cell::sync::Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent("AetherDesk/1.0")
        .build()
        .expect("PCGamingWiki HTTP client configuration must be valid")
});

fn is_valid_pcgw_drm_label(drm: &str) -> bool {
    let lower = drm.to_lowercase();
    !(lower.contains("team")
        || lower.contains("offline")
        || lower.contains("ffline")
        || lower.contains('=')
        || drm.len() > 30)
}

fn find_drm_template_value(wikitext: &str) -> Option<&str> {
    let lower = wikitext.to_ascii_lowercase();
    let mut offset = 0;
    while let Some(rel_start) = lower[offset..].find("{{drm") {
        let after_tag = offset + rel_start + 5;
        let trimmed = wikitext[after_tag..].trim_start();
        if let Some(after_pipe) = trimmed.strip_prefix('|') {
            let end = after_pipe.find("}}")?;
            return Some(&after_pipe[..end]);
        }
        offset = after_tag;
    }
    None
}

fn parse_pcgw_drm(wikitext: &str, app_id: u32) -> Option<String> {
    if !wikitext.contains(&app_id.to_string()) {
        return None;
    }
    let val = find_drm_template_value(wikitext)?.trim();
    let drm = if val.eq_ignore_ascii_case("none") || val.is_empty() {
        "None".to_string()
    } else {
        // Take first DRM name (before comma or pipe).
        val.split([',', '|'])
            .next()
            .unwrap_or(val)
            .trim()
            .to_string()
    };
    is_valid_pcgw_drm_label(&drm).then_some(drm)
}

fn extract_page_wikitext(page: &serde_json::Value) -> Option<&str> {
    page.get("revisions")
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
        .and_then(|r| r.get("*"))
        .and_then(|c| c.as_str())
}

fn select_pcgw_drm_from_pages(
    pages: &serde_json::Map<String, serde_json::Value>,
    ordered_titles: &[String],
    app_id: u32,
) -> Option<String> {
    for title in ordered_titles {
        for page in pages.values() {
            let page_title = page.get("title").and_then(|t| t.as_str()).unwrap_or("");
            if page_title.eq_ignore_ascii_case(title) {
                if let Some(drm) =
                    extract_page_wikitext(page).and_then(|wt| parse_pcgw_drm(wt, app_id))
                {
                    return Some(drm);
                }
            }
        }
    }
    for page in pages.values() {
        if let Some(drm) = extract_page_wikitext(page).and_then(|wt| parse_pcgw_drm(wt, app_id)) {
            return Some(drm);
        }
    }
    None
}

fn load_pcgw_cache_entries(
    cache_path: &std::path::Path,
) -> serde_json::Map<String, serde_json::Value> {
    let Ok(content) = std::fs::read_to_string(cache_path) else {
        return serde_json::Map::new();
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) else {
        return serde_json::Map::new();
    };
    parsed
        .get("entries")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default()
}

fn save_pcgw_cache_entries(
    cache_path: &std::path::Path,
    entries: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create PCGamingWiki cache dir: {e}"))?;
    }
    let stamped = serde_json::json!({ "entries": entries });
    let payload = serde_json::to_string(&stamped)
        .map_err(|e| format!("Failed to serialize PCGamingWiki cache: {e}"))?;
    let temp_path = cache_path.with_extension("tmp");
    std::fs::write(&temp_path, payload)
        .map_err(|e| format!("Failed to write PCGamingWiki cache: {e}"))?;
    std::fs::rename(&temp_path, cache_path)
        .map_err(|e| format!("Failed to apply PCGamingWiki cache: {e}"))
}

fn split_pcgw_cache_hits(
    entries: &serde_json::Map<String, serde_json::Value>,
    pairs: &[(u32, String)],
    now: u64,
) -> (HashMap<u32, String>, Vec<(u32, String)>) {
    let mut hits = HashMap::new();
    let mut need_fetch = Vec::new();
    for (id, name) in pairs {
        if let Some(entry) = entries.get(&id.to_string()) {
            let ts = entry.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
            if let Some(drm) = entry.get("drm").and_then(|v| v.as_str()) {
                let ttl = if drm.is_empty() {
                    PCGW_NEGATIVE_CACHE_TTL
                } else {
                    PCGW_POSITIVE_CACHE_TTL
                };
                if now.saturating_sub(ts) <= ttl {
                    if !drm.is_empty() {
                        hits.insert(*id, drm.to_string());
                    }
                    continue;
                }
            }
        }
        need_fetch.push((*id, name.clone()));
    }
    (hits, need_fetch)
}

async fn fetch_single_pcgw_drm(
    client: &reqwest::Client,
    id: u32,
    name: &str,
) -> Result<Option<String>, ()> {
    let search_url = format!(
        "https://www.pcgamingwiki.com/w/api.php?action=query&list=search&srsearch={}&srlimit=5&format=json",
        percent_encode(name)
    );
    let search_resp = client.get(&search_url).send().await.map_err(|_| ())?;
    if !search_resp.status().is_success() {
        return Err(());
    }
    let search_json = search_resp
        .json::<serde_json::Value>()
        .await
        .map_err(|_| ())?;
    let titles: Vec<String> = search_json
        .get("query")
        .and_then(|q| q.get("search"))
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten()
        .filter_map(|o| o.get("title").and_then(|t| t.as_str()).map(str::to_string))
        .take(5)
        .collect();
    if titles.is_empty() {
        return Ok(None);
    }

    let joined_titles = titles.join("|");
    let content_url = format!(
        "https://www.pcgamingwiki.com/w/api.php?action=query&prop=revisions&rvprop=content&titles={}&format=json",
        percent_encode(&joined_titles)
    );
    let content_resp = client.get(&content_url).send().await.map_err(|_| ())?;
    if !content_resp.status().is_success() {
        return Err(());
    }
    let content_json = content_resp
        .json::<serde_json::Value>()
        .await
        .map_err(|_| ())?;
    let Some(pages) = content_json
        .get("query")
        .and_then(|q| q.get("pages"))
        .and_then(|p| p.as_object())
    else {
        return Ok(None);
    };

    Ok(select_pcgw_drm_from_pages(pages, &titles, id))
}

pub async fn check_pc_drm_bulk(
    _app: &tauri::AppHandle,
    app_ids: Vec<u32>,
    names: Vec<String>,
) -> Result<HashMap<u32, String>, String> {
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    let cache_dir = LocalAppPaths::data_root().join("cache");
    let cache_path = cache_dir.join("pc_drm_cache.json");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let pairs: Vec<(u32, String)> = app_ids.into_iter().zip(names).collect();
    let mut entries = load_pcgw_cache_entries(&cache_path);
    let (mut result, need_fetch) = split_pcgw_cache_hits(&entries, &pairs, now);

    if need_fetch.is_empty() {
        return Ok(result);
    }

    let semaphore = Arc::new(tokio::sync::Semaphore::new(PCGW_CONCURRENCY_LIMIT));
    let mut join_set = tokio::task::JoinSet::new();
    for (id, name) in need_fetch {
        let permit_sem = Arc::clone(&semaphore);
        join_set.spawn(async move {
            let _permit = permit_sem.acquire_owned().await.ok();
            let outcome = fetch_single_pcgw_drm(&PCGW_CLIENT, id, &name).await;
            (id, outcome)
        });
    }

    let mut cache_dirty = false;
    while let Some(joined) = join_set.join_next().await {
        let Ok((id, outcome)) = joined else {
            continue;
        };
        match outcome {
            Ok(Some(drm)) => {
                entries.insert(id.to_string(), serde_json::json!({"drm": drm, "ts": now}));
                result.insert(id, drm);
                cache_dirty = true;
            }
            Ok(None) => {
                // Negative hit: cache for 24h so missing titles are not re-queried every render.
                entries.insert(id.to_string(), serde_json::json!({"drm": "", "ts": now}));
                cache_dirty = true;
            }
            Err(()) => {
                // Transient transport/HTTP error: never poison the cache.
            }
        }
    }

    if cache_dirty {
        if let Err(err) = save_pcgw_cache_entries(&cache_path, &entries) {
            crate::desk_log_warn!("drm", "Failed to save PCGamingWiki cache: {}", err);
        }
    }

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
        build_store_cache_key, build_trending_cache_key, load_pcgw_cache_entries,
        parse_crack_status_rows, parse_pcgw_drm, save_pcgw_cache_entries,
        select_pcgw_drm_from_pages, split_crack_cache_hits, split_pcgw_cache_hits,
        warm_store_front_with, CRACK_CACHE_TTL, PCGW_NEGATIVE_CACHE_TTL, STORE_FRONT_FIRST_PAGE,
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

        assert_eq!(
            *requested.lock().unwrap(),
            Some((0, STORE_FRONT_FIRST_PAGE))
        );
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

    #[test]
    fn parse_pcgw_drm_extracts_and_sanitizes_values() {
        let denuvo = "Steam AppID 12345 {{DRM|Denuvo, Steam}}";
        assert_eq!(parse_pcgw_drm(denuvo, 12345).as_deref(), Some("Denuvo"));

        let drm_free = "App 730 {{DRM|none}}";
        assert_eq!(parse_pcgw_drm(drm_free, 730).as_deref(), Some("None"));

        let wrong_id = "App 999 {{DRM|Denuvo}}";
        assert_eq!(parse_pcgw_drm(wrong_id, 730), None);

        let noisy = "App 730 {{DRM|offline team=1}}";
        assert_eq!(parse_pcgw_drm(noisy, 730), None);

        let spaced = "App 730 {{drm | Ubisoft Connect}}";
        assert_eq!(
            parse_pcgw_drm(spaced, 730).as_deref(),
            Some("Ubisoft Connect")
        );
    }

    #[test]
    fn select_pcgw_drm_from_pages_checks_beyond_first_candidate() {
        let mut pages = serde_json::Map::new();
        pages.insert(
            "1".to_string(),
            serde_json::json!({
                "title": "Modern Warfare 4 (2007)",
                "revisions": [{ "*": "App 7940 {{DRM|Steam}}" }]
            }),
        );
        pages.insert(
            "2".to_string(),
            serde_json::json!({
                "title": "Call of Duty: Modern Warfare 4",
                "revisions": [{ "*": "App 4435490 {{DRM|Denuvo, Steam}}" }]
            }),
        );
        let ordered = vec![
            "Modern Warfare 4 (2007)".to_string(),
            "Call of Duty: Modern Warfare 4".to_string(),
        ];

        assert_eq!(
            select_pcgw_drm_from_pages(&pages, &ordered, 4435490).as_deref(),
            Some("Denuvo")
        );
    }

    #[test]
    fn parse_crack_status_rows_accepts_numeric_and_string_appids() {
        let rows = vec![
            serde_json::json!({ "steam_appid": 100, "status": "cracked" }),
            serde_json::json!({ "steam_appid": "200", "status": "uncracked" }),
            serde_json::json!({ "steam_appid": "300", "status": " " }),
        ];
        let parsed = parse_crack_status_rows(&rows);
        assert_eq!(parsed.get(&100).map(|v| v.status.as_str()), Some("cracked"));
        assert_eq!(
            parsed.get(&200).map(|v| v.status.as_str()),
            Some("uncracked")
        );
        assert!(!parsed.contains_key(&300));
    }

    #[test]
    fn crack_cache_skips_fresh_negative_hit_without_returning_empty() {
        let mut entries = serde_json::Map::new();
        let now = 1_700_000_000;
        entries.insert(
            "10".to_string(),
            serde_json::json!({ "status": "", "ts": now - 100 }),
        );
        entries.insert(
            "20".to_string(),
            serde_json::json!({ "status": "", "ts": now - CRACK_CACHE_TTL - 10 }),
        );
        entries.insert(
            "30".to_string(),
            serde_json::json!({ "status": "cracked", "ts": now - 100 }),
        );

        let (hits, need_fetch) = split_crack_cache_hits(&entries, &[10, 20, 30], now);
        assert_eq!(hits.get(&30).map(|v| v.status.as_str()), Some("cracked"));
        assert!(!hits.contains_key(&10));
        assert_eq!(need_fetch, vec![20]);
    }

    #[test]
    fn pcgw_cache_skips_fresh_negative_hit_without_returning_empty() {
        let mut entries = serde_json::Map::new();
        let now = 1_700_000_000;
        entries.insert(
            "10".to_string(),
            serde_json::json!({ "drm": "", "ts": now - 100 }),
        );
        entries.insert(
            "20".to_string(),
            serde_json::json!({ "drm": "", "ts": now - PCGW_NEGATIVE_CACHE_TTL - 10 }),
        );
        entries.insert(
            "30".to_string(),
            serde_json::json!({ "drm": "Denuvo", "ts": now - 100 }),
        );

        let pairs = vec![
            (10, "Missing Fresh".to_string()),
            (20, "Missing Expired".to_string()),
            (30, "Denuvo Game".to_string()),
        ];
        let (hits, need_fetch) = split_pcgw_cache_hits(&entries, &pairs, now);
        assert_eq!(hits.get(&30).map(String::as_str), Some("Denuvo"));
        assert!(!hits.contains_key(&10));
        assert_eq!(need_fetch, vec![(20, "Missing Expired".to_string())]);
    }

    #[test]
    fn pcgw_cache_writes_atomically_without_leaving_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("pc_drm_cache.json");
        let mut entries = serde_json::Map::new();
        entries.insert(
            "730".to_string(),
            serde_json::json!({ "drm": "Steam", "ts": 123 }),
        );

        save_pcgw_cache_entries(&cache_path, &entries).unwrap();
        assert!(cache_path.is_file());
        assert!(!cache_path.with_extension("tmp").exists());

        let loaded = load_pcgw_cache_entries(&cache_path);
        assert_eq!(
            loaded
                .get("730")
                .and_then(|v| v.get("drm"))
                .and_then(|v| v.as_str()),
            Some("Steam")
        );
    }
}
