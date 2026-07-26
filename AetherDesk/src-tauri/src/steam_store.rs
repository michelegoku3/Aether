use serde::{Deserialize, Serialize};
use std::time::Duration;

const STEAM_SEARCH_TIMEOUT_SECONDS: u64 = 5;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SteamStoreItem {
    pub id: u32,
    pub name: String,
    #[serde(rename = "tiny_image")]
    pub image_url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SteamSearchResponse {
    pub total: u32,
    pub items: Vec<SteamStoreItem>,
}

pub struct SteamStore {
    client: reqwest::Client,
}

impl SteamStore {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(STEAM_SEARCH_TIMEOUT_SECONDS))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
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
