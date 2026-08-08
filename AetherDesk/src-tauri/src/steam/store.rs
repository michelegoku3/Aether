use serde::{Deserialize, Serialize};
use crate::providers::http;

const STEAM_SEARCH_TIMEOUT_SECONDS: u64 = 5;

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

    /// Queries Steam's official public Store Search API with automated percent-encoding
    pub async fn search_catalog(&self, query: &str) -> Result<Vec<SteamStoreItem>, String> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let url = "https://store.steampowered.com/api/storesearch/";
        
        let response = self.client.get(url)
            .query(&[("term", query), ("l", "italian"), ("cc", "IT")])
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
}
