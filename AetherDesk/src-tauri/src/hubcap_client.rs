use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};

const BASE_URL: &str = "https://hubcapmanifest.com/api/v1";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HubcapGameItem {
    // Deserialize App ID safely even if returned as a string or number in JSON, supporting game_id or appid aliases
    #[serde(alias = "game_id", alias = "appid", deserialize_with = "deserialize_app_id")]
    pub app_id: u32,
    #[serde(alias = "game_name", alias = "name")]
    pub name: String,
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
        serde_json::Value::Number(num) => {
            Ok(num.as_u64().unwrap_or(0) as u32)
        }
        serde_json::Value::String(s) => {
            s.parse::<u32>().map_err(serde::de::Error::custom)
        }
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
            client: reqwest::Client::new(),
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", self.api_key)) {
            headers.insert(AUTHORIZATION, value);
        }
        headers
    }

    /// Verifies if the configured Hubcap API key is valid by hitting the stats endpoint
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

    /// Downloads the `.lua` decrypted configuration file for a given App ID from Hubcap
    pub async fn download_lua_config(&self, app_id: u32) -> Result<String, String> {
        let url = format!("{}/lua/{}", BASE_URL, app_id);

        let response = self.client.get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| format!("Failed to send request: {}", e))?;

        if response.status().is_success() {
            let lua_content = response.text().await
                .map_err(|e| format!("Failed to read response body: {}", e))?;
            Ok(lua_content)
        } else {
            Err(format!("Failed to retrieve Lua. HTTP Status: {}", response.status()))
        }
    }

    /// Queries the Hubcap Manifest database for matches against a search term
    pub async fn search_library(&self, query: &str) -> Result<Vec<HubcapGameItem>, String> {
        let url = format!("{}/library", BASE_URL);

        let response = self.client.get(&url)
            .headers(self.headers())
            .query(&[("search", query), ("limit", "50")])
            .send()
            .await
            .map_err(|e| format!("Hubcap API network error: {}", e))?;

        if !response.status().is_success() {
            // If Hubcap key is invalid or fails, return empty list instead of crashing
            return Ok(Vec::new());
        }

        let data = response.json::<HubcapLibraryResponse>().await
            .map_err(|e| format!("Failed to parse Hubcap response: {}", e))?;

        Ok(data.games.unwrap_or_default())
    }
}
