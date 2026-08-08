use crate::providers::http;
use regex::Regex;
use serde::{Deserialize, Serialize};

const STEAM_SEARCH_TIMEOUT_SECONDS: u64 = 5;
const STEAM_SEARCH_RESULTS_URL: &str = "https://store.steampowered.com/search/results/";

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

#[derive(Clone)]
pub struct SteamStore {
    client: reqwest::Client,
}

impl SteamStore {
    pub fn new() -> Self {
        Self {
            client: http::build_client(STEAM_SEARCH_TIMEOUT_SECONDS),
        }
    }

    /// Queries Steam's official public Store Search API with an explicit country
    /// code so price currency can follow the user's settings without changing
    /// the caller's filtering/ranking semantics.
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
