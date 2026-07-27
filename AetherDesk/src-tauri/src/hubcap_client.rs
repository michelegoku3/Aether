use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read};
use std::time::Duration;
use zip::ZipArchive;

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
pub struct HubcapLibraryResponse {
    pub status: String,
    pub games: Option<Vec<HubcapGameItem>>,
}

#[derive(Debug, Clone)]
pub struct HubcapManifestFile {
    pub file_name: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct HubcapLuaPackage {
    pub lua_content: String,
    pub manifest_files: Vec<HubcapManifestFile>,
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

    /// Downloads the Hubcap manifest ZIP with a single API call and extracts:
    /// - the pinned Lua used by Aether/LumaCore
    /// - any `.manifest` files, ready to be copied to Steam/depotcache
    pub async fn download_lua_package(&self, app_id: u32) -> Result<HubcapLuaPackage, String> {
        let bytes = self.download_manifest_zip(app_id).await?;
        Self::extract_package_from_zip(app_id, bytes.as_ref())
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

    fn extract_package_from_zip(app_id: u32, bytes: &[u8]) -> Result<HubcapLuaPackage, String> {
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| format!("Failed to open manifest ZIP: {}", e))?;

        let preferred_name = format!("{}.lua", app_id);
        let mut preferred_lua: Option<String> = None;
        let mut first_lua: Option<String> = None;
        let mut manifest_files = Vec::new();

        for index in 0..archive.len() {
            let mut file = archive.by_index(index)
                .map_err(|e| format!("Failed to read ZIP entry {}: {}", index, e))?;
            let name = file.name().replace('\\', "/");
            let lower_name = name.to_ascii_lowercase();
            let file_name = name.rsplit('/').next().unwrap_or(&name).to_string();

            if lower_name.ends_with(".lua") {
                let mut content = String::new();
                file.read_to_string(&mut content)
                    .map_err(|e| format!("Failed to read Lua file from ZIP ({}): {}", name, e))?;

                let is_preferred = file_name.eq_ignore_ascii_case(&preferred_name);
                if is_preferred && Self::contains_setmanifestid(&content) {
                    preferred_lua = Some(content);
                } else if first_lua.is_none() && Self::contains_setmanifestid(&content) {
                    first_lua = Some(content);
                }
            } else if lower_name.ends_with(".manifest") {
                let mut manifest_bytes = Vec::new();
                file.read_to_end(&mut manifest_bytes)
                    .map_err(|e| format!("Failed to read manifest file from ZIP ({}): {}", name, e))?;
                manifest_files.push(HubcapManifestFile { file_name, bytes: manifest_bytes });
            }
        }

        let lua_content = preferred_lua
            .or(first_lua)
            .ok_or_else(|| "Manifest ZIP did not contain a Lua file with setManifestid pins".to_string())?;

        Ok(HubcapLuaPackage { lua_content, manifest_files })
    }

    fn contains_setmanifestid(content: &str) -> bool {
        content.to_ascii_lowercase().contains("setmanifestid")
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
