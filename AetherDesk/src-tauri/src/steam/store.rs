use crate::providers::http;
use regex::Regex;
use reqwest::header::USER_AGENT;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const STEAM_BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

const STEAM_SEARCH_TIMEOUT_SECONDS: u64 = 5;
const STEAM_SEARCH_RESULTS_URL: &str = "https://store.steampowered.com/search/results/";
const STEAM_SUGGEST_URL: &str = "https://store.steampowered.com/search/suggest";
const STEAM_SEARCH_APPS_URL: &str = "https://steamcommunity.com/actions/SearchApps";
const SUGGEST_CACHE_CAP: usize = 80;
const SUGGEST_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
/// How long the typeahead keeps waiting for `SearchApps` once `/search/suggest`
/// has answered. The ranked short list is what the user is waiting for; the
/// long tail only matters when scrolling past it, so a slow steamcommunity
/// round-trip must not hold the whole suggestion box back. A late tail is
/// still merged into the cache entry for the next identical query.
const SEARCH_APPS_GRACE: Duration = Duration::from_millis(250);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SteamStoreItem {
    pub id: u32,
    pub name: String,
    #[serde(rename = "tiny_image")]
    pub image_url: String,
    #[serde(default, rename = "type")]
    pub item_type: Option<String>,
    #[serde(default)]
    pub price: Option<SteamStorePrice>,
    #[serde(default)]
    pub metascore: Option<serde_json::Value>,
    #[serde(default)]
    pub platforms: Option<SteamStorePlatforms>,
    #[serde(default)]
    pub streamingvideo: Option<bool>,
    #[serde(default)]
    pub controller_support: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SteamStorePrice {
    pub currency: Option<String>,
    pub initial: Option<i64>,
    #[serde(default, rename = "final")]
    pub final_price: Option<i64>,
    #[serde(default)]
    pub discount_percent: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SteamStorePlatforms {
    #[serde(default)]
    pub windows: Option<bool>,
    #[serde(default)]
    pub mac: Option<bool>,
    #[serde(default)]
    pub linux: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SteamSearchResponse {
    pub total: u32,
    pub items: Vec<SteamStoreItem>,
}

#[derive(Debug, Deserialize)]
struct SteamSearchResultsResponse {
    success: Option<u32>,
    results_html: Option<String>,
}

/// Community SearchApps JSON. `appid` is sometimes a string, sometimes a number.
#[derive(Debug, Deserialize)]
struct SteamSearchApp {
    #[serde(deserialize_with = "deserialize_appid_as_string")]
    appid: String,
    name: String,
    #[serde(default)]
    logo: Option<String>,
}

fn deserialize_appid_as_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        _ => Err(serde::de::Error::custom("appid must be a string or number")),
    }
}

#[derive(Clone)]
pub struct SteamStore {
    client: reqwest::Client,
}

/// Process-lifetime HTTP client for the Steam storefront endpoints.
///
/// The typeahead fires one request per debounced keystroke and every store
/// search builds a fresh `StoreService`; constructing a `reqwest::Client` per
/// call meant a new TCP + TLS handshake to store.steampowered.com /
/// steamcommunity.com every time (a few hundred ms each from Europe) and no
/// connection reuse at all. `Client` is an `Arc` inside, so sharing one keeps
/// the pool warm and makes cloning free.
fn shared_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| http::build_client(STEAM_SEARCH_TIMEOUT_SECONDS))
        .clone()
}

impl SteamStore {
    pub fn new() -> Self {
        Self {
            client: shared_client(),
        }
    }

    /// Autocomplete for the Store typeahead.
    ///
    /// Steam `/search/suggest` returns a short ranked list (~5). We merge it
    /// with `SearchApps` (longer list) so the UI can scroll past the first five
    /// without losing Steam's ranking on top. Identical queries are served from
    /// a process-lifetime LRU and never hit Steam twice.
    pub async fn suggest_for_country(&self, query: &str, country_code: &str) -> Result<Vec<SteamStoreItem>, String> {
        let trimmed = query.trim();
        if trimmed.len() < 2 {
            return Ok(Vec::new());
        }

        let cache_key = suggest_cache_key(country_code, trimmed);
        if let Some(cached) = suggest_cache_get(&cache_key) {
            return Ok(cached);
        }

        // Both endpoints start together; `/search/suggest` (the ranked short
        // list) decides when the answer goes back to the UI, `SearchApps` (the
        // long tail) gets a short grace and otherwise finishes in the
        // background, enriching the cache entry for the next identical query.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            // No Tokio runtime to detach onto (only possible outside the Tauri
            // async runtime): fall back to the plain fan-out.
            let (suggest_res, apps_res) = tokio::join!(
                self.fetch_suggest_html(trimmed, country_code),
                fetch_search_apps_with(&self.client, trimmed),
            );
            return Ok(suggest_cache_merge(
                &cache_key,
                suggest_res.unwrap_or_default(),
                apps_res.unwrap_or_default(),
            ));
        };

        let (tail_tx, tail_rx) = tokio::sync::oneshot::channel::<Vec<SteamStoreItem>>();
        {
            let client = self.client.clone();
            let query = trimmed.to_string();
            let cache_key = cache_key.clone();
            runtime.spawn(async move {
                let apps = fetch_search_apps_with(&client, &query)
                    .await
                    .unwrap_or_default();
                if let Err(apps) = tail_tx.send(apps) {
                    // The caller stopped waiting: merge the late tail so the
                    // cached entry still grows to the full list.
                    suggest_cache_merge(&cache_key, Vec::new(), apps);
                }
            });
        }

        let suggest_items = self
            .fetch_suggest_html(trimmed, country_code)
            .await
            .unwrap_or_default();
        let apps_items = match tokio::time::timeout(SEARCH_APPS_GRACE, tail_rx).await {
            Ok(Ok(apps)) => apps,
            _ => Vec::new(),
        };

        Ok(suggest_cache_merge(&cache_key, suggest_items, apps_items))
    }

    async fn fetch_suggest_html(&self, query: &str, country_code: &str) -> Result<Vec<SteamStoreItem>, String> {
        let response = self
            .client
            .get(STEAM_SUGGEST_URL)
            .header(USER_AGENT, STEAM_BROWSER_UA)
            .query(&[
                ("term", query),
                ("f", "games"),
                ("cc", country_code),
                ("l", "italian"),
                ("realm", "1"),
            ])
            .send()
            .await
            .map_err(|e| format!("Steam suggest request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Steam suggest returned HTTP {}", response.status()));
        }

        let html = response
            .text()
            .await
            .map_err(|e| format!("Failed to read Steam suggest body: {}", e))?;
        Ok(parse_suggest_html(&html))
    }


    pub async fn search_catalog_for_country(&self, query: &str, country_code: &str) -> Result<Vec<SteamStoreItem>, String> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let url = "https://store.steampowered.com/api/storesearch/";
        
        let response = self.client.get(url)
            .query(&[("term", query), ("l", "italian"), ("cc", country_code)])
            .send()
            .await
            .map_err(|e| format!("Steam API network error: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Steam returned HTTP error: {}", response.status()));
        }

        let data = response.json::<SteamSearchResponse>().await
            .map_err(|e| format!("Failed to parse Steam response: {}", e))?;

        Ok(data.items)
    }

    /// Fetches Steam's trending released games using the same JSON-backed
    /// endpoint that powers the public search page. The endpoint returns HTML
    /// rows inside JSON, so parsing is intentionally isolated here instead of
    /// leaking scrape details into StoreService.
    pub async fn store_front_for_country(
        &self,
        filter: &str,
        start: usize,
        count: usize,
        country_code: &str,
    ) -> Result<Vec<SteamStoreItem>, String> {
        let params = store_front_params(filter, start, count, country_code);
        let response = self
            .client
            .get(STEAM_SEARCH_RESULTS_URL)
            .query(&params)
            .send()
            .await
            .map_err(|e| format!("Steam trending request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Steam trending returned HTTP {}", response.status()));
        }

        let data = response
            .json::<SteamSearchResultsResponse>()
            .await
            .map_err(|e| format!("Failed to parse Steam trending response: {}", e))?;

        if data.success != Some(1) {
            return Ok(Vec::new());
        }

        Ok(parse_search_results_html(
            data.results_html.as_deref().unwrap_or_default(),
            country_code,
        ))
    }
}


pub fn parse_suggest_html(html: &str) -> Vec<SteamStoreItem> {
    let Ok(row_re) = Regex::new(
        r#"(?s)data-ds-appid="(?P<appid>\d+)"(?P<body>.*?)(?:</a>|$)"#,
    ) else {
        return Vec::new();
    };
    let name_re = Regex::new(r#"(?s)class="match_name"[^>]*>(?P<name>.*?)</div>"#).ok();
    let img_re = Regex::new(r#"<img\s+src="(?P<src>[^"]+)""#).ok();
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();

    for captures in row_re.captures_iter(html) {
        let Some(id) = captures.name("appid").and_then(|m| m.as_str().parse::<u32>().ok()) else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        let body = captures.name("body").map(|m| m.as_str()).unwrap_or("");
        let name = name_re
            .as_ref()
            .and_then(|re| re.captures(body))
            .and_then(|caps| caps.name("name"))
            .map(|m| decode_html(m.as_str()))
            .filter(|title| !title.trim().is_empty());
        let Some(name) = name else { continue };
        let image_url = img_re
            .as_ref()
            .and_then(|re| re.captures(body))
            .and_then(|caps| caps.name("src"))
            .map(|m| decode_html(m.as_str()))
            .unwrap_or_default();

        items.push(SteamStoreItem {
            id,
            name,
            image_url,
            item_type: Some("app".to_string()),
            price: None,
            metascore: None,
            platforms: None,
            streamingvideo: None,
            controller_support: None,
        });
    }

    items
}

fn urlencoding_path(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            ' ' => out.push_str("%20"),
            _ => {
                for byte in ch.encode_utf8(&mut [0; 4]).as_bytes() {
                    out.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    out
}

fn store_front_params(
    filter: &str,
    start: usize,
    count: usize,
    country_code: &str,
) -> Vec<(&'static str, String)> {
    let mut params = vec![
        ("start", start.to_string()),
        ("count", count.min(100).to_string()),
        ("dynamic_data", String::new()),
        ("infinite", "1".to_string()),
        ("json", "1".to_string()),
        ("cc", country_code.to_string()),
        ("l", "italian".to_string()),
        // Games only. Avoids hardware/videos/tools leaking into the Store front.
        ("category1", "998".to_string()),
    ];

    match filter {
        "latest" => params.push(("sort_by", "Released_DESC".to_string())),
        "top_sellers" => params.push(("filter", "topsellers".to_string())),
        "upcoming" => {
            params.push(("filter", "comingsoon".to_string()));
            params.push(("sort_by", "Released_ASC".to_string()));
        }
        "popular_upcoming" => params.push(("filter", "popularcomingsoon".to_string())),
        "discounts" => params.push(("specials", "1".to_string())),
        // `trending` and `trendingreleased` currently resolve to the same public
        // Steam Search JSON feed. Prefer the shorter `trending` name exposed in Settings.
        _ => params.push(("filter", "trending".to_string())),
    }

    params
}

fn parse_search_results_html(html: &str, country_code: &str) -> Vec<SteamStoreItem> {
    let Ok(row_re) = Regex::new(
        r#"(?s)<a\b[^>]*data-ds-appid=\"(?P<appid>\d+)\"[^>]*class=\"search_result_row[^\"]*\"[^>]*>(?P<body>.*?)</a>"#,
    ) else {
        return Vec::new();
    };
    let img_re = Regex::new(r#"<img\s+src=\"(?P<src>[^\"]+)\""#).ok();
    let title_re = Regex::new(r#"(?s)<span\s+class=\"title\">(?P<title>.*?)</span>"#).ok();
    let price_re = Regex::new(r#"data-price-final=\"(?P<price>\d+)\""#).ok();

    row_re
        .captures_iter(html)
        .filter_map(|captures| {
            let id = captures.name("appid")?.as_str().parse::<u32>().ok()?;
            let body = captures.name("body")?.as_str();
            let name = title_re
                .as_ref()
                .and_then(|re| re.captures(body))
                .and_then(|caps| caps.name("title"))
                .map(|m| decode_html(m.as_str()))
                .filter(|title| !title.trim().is_empty())?;
            let image_url = img_re
                .as_ref()
                .and_then(|re| re.captures(body))
                .and_then(|caps| caps.name("src"))
                .map(|m| decode_html(m.as_str()))
                .unwrap_or_default();
            let final_price = price_re
                .as_ref()
                .and_then(|re| re.captures(body))
                .and_then(|caps| caps.name("price"))
                .and_then(|m| m.as_str().parse::<i64>().ok());

            Some(SteamStoreItem {
                id,
                name,
                image_url,
                item_type: Some("app".to_string()),
                price: final_price.map(|price| SteamStorePrice {
                    currency: Some(currency_for_country(country_code).to_string()),
                    initial: None,
                    final_price: Some(price),
                    discount_percent: None,
                }),
                metascore: None,
                platforms: Some(SteamStorePlatforms {
                    windows: Some(body.contains("platform_img win")),
                    mac: Some(body.contains("platform_img mac")),
                    linux: Some(body.contains("platform_img linux")),
                }),
                streamingvideo: None,
                controller_support: None,
            })
        })
        .collect()
}

fn currency_for_country(country_code: &str) -> &'static str {
    match country_code.trim().to_uppercase().as_str() {
        "US" => "USD",
        "JP" => "JPY",
        _ => "EUR",
    }
}

/// `steamcommunity.com/actions/SearchApps/<query>`: the long, unranked
/// autocomplete list. Free function (not a method) so the typeahead can run it
/// on a detached task that outlives the caller's grace window.
async fn fetch_search_apps_with(client: &reqwest::Client, query: &str) -> Result<Vec<SteamStoreItem>, String> {
    let encoded = urlencoding_path(query);
    let url = format!("{}/{}", STEAM_SEARCH_APPS_URL, encoded);
    let response = client
        .get(&url)
        .header(USER_AGENT, STEAM_BROWSER_UA)
        .send()
        .await
        .map_err(|e| format!("Steam SearchApps request failed: {}", e))?;

    if !response.status().is_success() {
        return Ok(Vec::new());
    }

    let apps: Vec<SteamSearchApp> = response.json().await.unwrap_or_default();
    Ok(apps
        .into_iter()
        .filter_map(|app| {
            let id = app.appid.parse::<u32>().ok()?;
            let name = app.name.trim().to_string();
            if name.is_empty() {
                return None;
            }
            Some(SteamStoreItem {
                id,
                name,
                image_url: app.logo.unwrap_or_default(),
                item_type: Some("app".to_string()),
                price: None,
                metascore: None,
                platforms: None,
                streamingvideo: None,
                controller_support: None,
            })
        })
        .collect())
}

fn suggest_cache_key(country_code: &str, query: &str) -> String {
    format!(
        "{}:{}",
        country_code.trim().to_uppercase(),
        query.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
    )
}

struct SuggestCache {
    entries: HashMap<String, (Instant, Vec<SteamStoreItem>)>,
    order: VecDeque<String>,
}

fn suggest_cache() -> &'static Mutex<SuggestCache> {
    static CACHE: OnceLock<Mutex<SuggestCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(SuggestCache {
            entries: HashMap::new(),
            order: VecDeque::new(),
        })
    })
}

fn suggest_cache_get(key: &str) -> Option<Vec<SteamStoreItem>> {
    let mut cache = suggest_cache().lock().ok()?;
    let (stored_at, items) = cache.entries.get(key)?.clone();
    if stored_at.elapsed() > SUGGEST_CACHE_TTL {
        cache.entries.remove(key);
        cache.order.retain(|existing| existing != key);
        return None;
    }
    if let Some(index) = cache.order.iter().position(|existing| existing == key) {
        cache.order.remove(index);
        cache.order.push_back(key.to_string());
    }
    Some(items)
}

/// Merges a (possibly partial) suggestion batch into the cache entry for
/// `key` and returns the merged list. Order is stable regardless of which
/// half arrives first: Steam's ranked `suggest` rows, then whatever the entry
/// already held, then the `SearchApps` tail — deduplicated by app id. Called
/// by the typeahead path and by the detached tail task, so a slow tail
/// enriches the entry instead of being lost.
pub(crate) fn suggest_cache_merge(
    key: &str,
    ranked: Vec<SteamStoreItem>,
    tail: Vec<SteamStoreItem>,
) -> Vec<SteamStoreItem> {
    let Ok(mut cache) = suggest_cache().lock() else {
        let mut merged = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for item in ranked.into_iter().chain(tail) {
            if seen.insert(item.id) {
                merged.push(item);
            }
        }
        return merged;
    };
    let existing = cache
        .entries
        .get(key)
        .filter(|(stored_at, _)| stored_at.elapsed() <= SUGGEST_CACHE_TTL)
        .map(|(_, items)| items.clone())
        .unwrap_or_default();
    let mut merged = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in ranked.into_iter().chain(existing).chain(tail) {
        if seen.insert(item.id) {
            merged.push(item);
        }
    }
    if cache
        .entries
        .insert(key.to_string(), (Instant::now(), merged.clone()))
        .is_none()
    {
        cache.order.push_back(key.to_string());
    }
    while cache.order.len() > SUGGEST_CACHE_CAP {
        if let Some(oldest) = cache.order.pop_front() {
            cache.entries.remove(&oldest);
        }
    }
    merged
}

fn decode_html(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_string()
}
