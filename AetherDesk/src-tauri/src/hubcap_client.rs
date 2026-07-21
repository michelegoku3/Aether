use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};

const BASE_URL: &str = "https://hubcapmanifest.com/api/v1";

#[derive(Debug, Serialize, Deserialize, Clone)]
#[allow(dead_code)]
pub struct HubcapGame {
    pub app_id: u32,
    pub name: String,
    pub last_updated: Option<String>,
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
}
