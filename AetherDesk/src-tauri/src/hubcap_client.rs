use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read};
use zip::ZipArchive;

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
        headers.insert(USER_AGENT, HeaderValue::from_static("AetherDesk/1.0"));
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

    /// Downloads the full Lua configuration for a given App ID from Hubcap.
    ///
    /// We intentionally use only `/manifest/{appid}`. The `/lua/{appid}` endpoint can
    /// return a Lua without `setManifestid(...)` pins for some games, while the manifest
    /// ZIP is the same source used by the website download and contains the full pinned Lua.
    ///
    /// This is a single API call per game download: no `/lua` first attempt, no fallback.
    pub async fn download_lua_config(&self, app_id: u32) -> Result<String, String> {
        self.download_lua_from_manifest_zip(app_id).await
    }

    async fn download_lua_from_manifest_zip(&self, app_id: u32) -> Result<String, String> {
        let url = format!("{}/manifest/{}", BASE_URL, app_id);

        let response = self.client.get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| format!("Failed to send manifest ZIP request: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Failed to retrieve manifest ZIP. HTTP Status: {}", response.status()));
        }

        let bytes = response.bytes().await
            .map_err(|e| format!("Failed to read manifest ZIP bytes: {}", e))?;

        Self::extract_best_lua_from_zip(app_id, bytes.as_ref())
    }

    fn extract_best_lua_from_zip(app_id: u32, bytes: &[u8]) -> Result<String, String> {
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| format!("Failed to open manifest ZIP: {}", e))?;

        let preferred_name = format!("{}.lua", app_id);
        let mut first_lua: Option<String> = None;

        for index in 0..archive.len() {
            let mut file = archive.by_index(index)
                .map_err(|e| format!("Failed to read ZIP entry {}: {}", index, e))?;
            let name = file.name().replace('\\', "/");
            if !name.to_ascii_lowercase().ends_with(".lua") {
                continue;
            }

            let mut content = String::new();
            file.read_to_string(&mut content)
                .map_err(|e| format!("Failed to read Lua file from ZIP ({}): {}", name, e))?;

            let is_preferred = name
                .rsplit('/')
                .next()
                .map(|file_name| file_name.eq_ignore_ascii_case(&preferred_name))
                .unwrap_or(false);

            if is_preferred && Self::contains_setmanifestid(&content) {
                return Ok(content);
            }

            if Self::contains_setmanifestid(&content) {
                return Ok(content);
            }

            first_lua.get_or_insert(content);
        }

        first_lua.ok_or_else(|| "Manifest ZIP did not contain any .lua file".to_string()).and_then(|content| {
            if Self::contains_setmanifestid(&content) {
                Ok(content)
            } else {
                Err("Manifest ZIP Lua also does not contain setManifestid pins".to_string())
            }
        })
    }

    fn contains_setmanifestid(content: &str) -> bool {
        content.to_ascii_lowercase().contains("setmanifestid")
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
