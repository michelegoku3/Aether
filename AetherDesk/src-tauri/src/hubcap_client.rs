use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use crate::manifest_package::{ManifestPackage, ManifestPackageExtractor};

const BASE_URL: &str = "https://hubcapmanifest.com/api/v1";
const HUBCAP_TIMEOUT_SECONDS: u64 = 8;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HubcapGameItem {
    #[serde(alias = "game_id", alias = "appid", deserialize_with = "deserialize_app_id")]
    pub app_id: u32,
    #[serde(alias = "game_name", alias = "name")]
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HubcapUserStats {
    pub daily_usage: Option<u32>,
    pub role_daily_limit: Option<u32>,
    pub daily_limit: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HubcapLibraryResponse {
    pub status: String,
    pub games: Option<Vec<HubcapGameItem>>,
}

fn deserialize_app_id<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(num) => Ok(num.as_u64().unwrap_or(0) as u32),
        serde_json::Value::String(s) => s.parse::<u32>().map_err(serde::de::Error::custom),
        _ => Err(serde::de::Error::custom("Invalid App ID type")),
    }
}

pub struct HubcapClient {
    api_key: String,
    client: reqwest::Client,
}

impl HubcapClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(HUBCAP_TIMEOUT_SECONDS))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", self.api_key)) {
            headers.insert(AUTHORIZATION, value);
        }
        headers.insert(USER_AGENT, HeaderValue::from_static("AetherDesk/1.0"));
        headers
    }

    pub async fn validate_api_key(&self) -> Result<bool, String> {
        let url = format!("{}/user/stats", BASE_URL);
        let response = self.client.get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if response.status().is_success() {
            Ok(true)
        } else if response.status().as_u16() == 401 {
            Ok(false)
        } else {
            Err(format!("Server returned HTTP error: {}", response.status()))
        }
    }

    /// Downloads Hubcap's manifest ZIP with a single API call and delegates archive
    /// parsing to the provider-agnostic `ManifestPackageExtractor`.
    pub async fn download_lua_package(&self, app_id: u32) -> Result<ManifestPackage, String> {
        let bytes = self.download_manifest_zip(app_id).await?;
        ManifestPackageExtractor::from_zip(app_id, bytes.as_ref())
    }

    async fn download_manifest_zip(&self, app_id: u32) -> Result<Vec<u8>, String> {
        let url = format!("{}/manifest/{}", BASE_URL, app_id);
        let response = self.client.get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| format!("Failed to send manifest ZIP request: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Failed to retrieve manifest ZIP. HTTP Status: {}", response.status()));
        }

        response.bytes().await
            .map(|bytes| bytes.to_vec())
            .map_err(|e| format!("Failed to read manifest ZIP bytes: {}", e))
    }

    pub async fn get_usage_stats(&self) -> Result<HubcapUserStats, String> {
        let url = format!("{}/user/stats", BASE_URL);
        let response = self.client.get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if response.status().is_success() {
            let stats = response.json::<HubcapUserStats>().await
                .map_err(|e| format!("Failed to parse user stats: {}", e))?;
            Ok(stats)
        } else {
            Err(format!("Server returned HTTP error ({}): {}", response.status(), response.status().canonical_reason().unwrap_or("Unknown")))
        }
    }

    pub async fn search_library(&self, query: &str) -> Result<Vec<HubcapGameItem>, String> {
        let url = format!("{}/library", BASE_URL);
        let response = self.client.get(&url)
            .headers(self.headers())
            .query(&[("search", query), ("limit", "50")])
            .send()
            .await
            .map_err(|e| format!("Hubcap API network error: {}", e))?;

        if !response.status().is_success() {
            return Ok(Vec::new());
        }

        let data = response.json::<HubcapLibraryResponse>().await
            .map_err(|e| format!("Failed to parse Hubcap response: {}", e))?;

        Ok(data.games.unwrap_or_default())
    }
}
