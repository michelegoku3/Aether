use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use crate::manifest::package::{ManifestPackage, ManifestPackageExtractor};
use crate::providers::http;
use crate::providers::hubcap_generation::{
    deduplicated_generation, GenerationKey, GenerationKind,
};

const BASE_URL: &str = "https://hubcapmanifest.com/api/v1";
const HUBCAP_TIMEOUT_SECONDS: u64 = 8;

/// `/library` gets a moderate page: bigger pages mean bigger payloads and a
/// longer Hubcap-only tail that the pre-filter then has to cut down anyway.
const LIBRARY_SEARCH_LIMIT: u32 = 100;
const CATALOG_SEARCH_LIMIT: u32 = 50;

fn encode_query_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

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
    pub can_make_requests: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HubcapGenerationBucket {
    pub usage: Option<u32>,
    pub limit: Option<u32>,
    pub remaining: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HubcapGenerationUsage {
    #[serde(default)]
    pub single: HubcapGenerationBucket,
    #[serde(default)]
    pub bundle: HubcapGenerationBucket,
    #[serde(default)]
    pub workshop: HubcapGenerationBucket,
    pub steam_service_ready: Option<bool>,
}

/// Envelope of `GET /library`. Every field is optional: Hubcap's payload shape
/// has shifted over time (SFF parses it defensively with `data.get(...)`), and a
/// single missing key must never turn the whole search into "no results".
#[derive(Debug, Deserialize)]
struct HubcapLibraryResponse {
    #[serde(default)]
    games: Vec<HubcapGameItem>,
}

/// Envelope of `GET /search`. The endpoint has shipped several payload shapes
/// depending on version (`{"results": [...]}`, a bare `[...]`, and the
/// `/library`-style `{"games": [...]}` — all handled defensively by SFF), so
/// accept any of them; extra keys are ignored.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum HubcapSearchResponse {
    WrappedResults { results: Vec<HubcapGameItem> },
    WrappedGames { games: Vec<HubcapGameItem> },
    Bare(Vec<HubcapGameItem>),
}

impl HubcapSearchResponse {
    fn into_items(self) -> Vec<HubcapGameItem> {
        match self {
            Self::WrappedResults { results } => results,
            Self::WrappedGames { games } => games,
            Self::Bare(items) => items,
        }
    }
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

#[derive(Clone)]
pub struct HubcapClient {
    api_key: String,
    client: reqwest::Client,
}

impl HubcapClient {
    pub fn new(api_key: String) -> Self {
        let api_key = api_key.trim().to_string();
        let mut headers = HeaderMap::new();
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", api_key)) {
            headers.insert(AUTHORIZATION, value);
        }
        Self {
            api_key,
            client: http::build_client_with_headers(HUBCAP_TIMEOUT_SECONDS, headers),
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", self.api_key)) {
            headers.insert(AUTHORIZATION, value);
        }
        headers
    }

    /// Server-side statuses that mean "this query is not answerable" rather than
    /// "the app is broken". Hubcap 400s many cyrillic queries and has known
    /// 500/503 clusters; all of them must surface as an empty result set so the
    /// rest of the pipeline (Steam catalog, the other endpoint) keeps working.
    fn is_soft_failure(status: reqwest::StatusCode) -> bool {
        matches!(status.as_u16(), 400 | 500 | 503)
    }

    pub async fn validate_api_key(&self) -> Result<bool, String> {
        crate::desk_log_info!("hubcap", "Validating API key with {}/user/stats", BASE_URL);
        let url = format!("{}/user/stats", BASE_URL);
        let response = self.client.get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| {
                crate::desk_log_error!("hubcap", "Network error validating API key: {}", e);
                format!("Network error: {}", e)
            })?;

        if response.status().is_success() {
            let stats = response
                .json::<HubcapUserStats>()
                .await
                .map_err(|e| format!("Failed to parse Hubcap account status: {e}"))?;
            let active = stats.can_make_requests.unwrap_or(true);
            if active {
                crate::desk_log_info!("hubcap", "API key validated successfully (active account)");
            } else {
                crate::desk_log_warn!("hubcap", "Hubcap account is authenticated but cannot make requests");
            }
            Ok(active)
        } else if response.status().as_u16() == 401 {
            crate::desk_log_warn!("hubcap", "API key validation returned 401 Unauthorized");
            Ok(false)
        } else {
            crate::desk_log_error!("hubcap", "API key validation server error: HTTP {}", response.status());
            Err(format!("Server returned HTTP error: {}", response.status()))
        }
    }

    /// Downloads Hubcap's manifest ZIP with a single API call and delegates archive
    /// parsing to the provider-agnostic `ManifestPackageExtractor`.
    pub async fn download_lua_package(&self, app_id: u32) -> Result<ManifestPackage, String> {
        let bytes = deduplicated_generation(
            GenerationKey::AppBundle {
                app_id,
                branch: "manifest".to_string(),
            },
            GenerationKind::Game,
            || self.download_manifest_zip(app_id),
        )
        .await?;
        ManifestPackageExtractor::from_zip(app_id, bytes.as_ref())
    }

    async fn download_manifest_zip(&self, app_id: u32) -> Result<Vec<u8>, String> {
        let stats = self.get_usage_stats().await?;
        if stats.can_make_requests == Some(false) {
            return Err("Hubcap account is not allowed to make manifest requests.".to_string());
        }
        let limit = stats.role_daily_limit.or(stats.daily_limit);
        if let (Some(usage), Some(limit)) = (stats.daily_usage, limit) {
            if usage >= limit {
                return Err(format!("Hubcap account daily manifest quota is exhausted ({usage}/{limit})."));
            }
        }
        crate::desk_log_info!("hubcap", "Requesting Hubcap manifest ZIP for AppID {} from {}", app_id, BASE_URL);
        let url = format!("{}/manifest/{}", BASE_URL, app_id);
        let response = self.client.get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| {
                crate::desk_log_error!("hubcap", "Network error requesting Hubcap manifest ZIP for AppID {}: {}", app_id, e);
                format!("Failed to send manifest ZIP request: {}", e)
            })?;

        if !response.status().is_success() {
            crate::desk_log_error!("hubcap", "Hubcap manifest ZIP request for AppID {} failed with HTTP status {}", app_id, response.status());
            return Err(format!("Failed to retrieve manifest ZIP. HTTP Status: {}", response.status()));
        }

        let bytes = response.bytes().await
            .map(|bytes| bytes.to_vec())
            .map_err(|e| {
                crate::desk_log_error!("hubcap", "Failed to read Hubcap manifest ZIP bytes for AppID {}: {}", app_id, e);
                format!("Failed to read manifest ZIP bytes: {}", e)
            })?;
        crate::desk_log_info!("hubcap", "Downloaded Hubcap manifest ZIP for AppID {} successfully ({} bytes)", app_id, bytes.len());
        Ok(bytes)
    }

    /// Lightweight existence check: does Hubcap have a manifest for this `app_id`?
    /// Uses `GET /status/{id}` (Free - No usage count per Api Endpoints.txt),
    /// never `GET /manifest/{id}` which *counts* toward daily usage.
    /// Interprets `manifest_file_exists == true` or `status == "available"` as true.
    /// Any non-200, parse error, or network failure is `false` (fail-open).
    pub async fn has_manifest(&self, app_id: u32) -> bool {
        let url = format!("{}/status/{}", BASE_URL, app_id);
        match self.client.get(&url).headers(self.headers()).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<serde_json::Value>().await {
                    Ok(v) => {
                        if let Some(b) = v.get("manifest_file_exists").and_then(|x| x.as_bool()) {
                            if b { return true; }
                        }
                        if let Some(s) = v.get("status").and_then(|x| x.as_str()) {
                            if s.eq_ignore_ascii_case("available") { return true; }
                        }
                        if let Some(b) = v.get("available").and_then(|x| x.as_bool()) { return b; }
                        if let Some(b) = v.get("exists").and_then(|x| x.as_bool()) { return b; }
                        false
                    }
                    Err(_) => true,
                }
            }
            Ok(_) => false,
            Err(_) => false,
        }
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

    /// Returns the server-side generation quota and Steam readiness. This is a
    /// free endpoint and is checked immediately before every generation request;
    /// the local scheduler still enforces the larger application-level caps.
    pub async fn get_generation_usage(&self) -> Result<HubcapGenerationUsage, String> {
        let url = format!("{}/generate/usage", BASE_URL);
        let response = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| format!("Hubcap generation-usage request failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("Hubcap generation usage returned HTTP {}", response.status()));
        }
        response
            .json::<HubcapGenerationUsage>()
            .await
            .map_err(|e| format!("Failed to parse Hubcap generation usage: {e}"))
    }

    async fn ensure_generation_available(
        &self,
        bucket_name: &str,
        remaining: Option<u32>,
        usage: &HubcapGenerationUsage,
    ) -> Result<(), String> {
        if usage.steam_service_ready == Some(false) {
            return Err("Hubcap reports that the Steam service is not ready for generation.".to_string());
        }
        if remaining == Some(0) {
            return Err(format!("Hubcap {bucket_name} generation quota is exhausted."));
        }
        Ok(())
    }

    async fn get_generated_bytes(&self, url: String, description: &str) -> Result<Vec<u8>, String> {
        let response = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| format!("Hubcap {description} request failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("Hubcap {description} request returned HTTP {}", response.status()));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read Hubcap {description} bytes: {e}"))?
            .to_vec();
        if bytes.is_empty() {
            return Err(format!("Hubcap returned an empty {description}."));
        }
        // Generation endpoints return binary data. Surface a useful provider
        // error instead of writing a JSON error document as a .manifest file.
        if content_type.contains("json") || content_type.starts_with("text/") {
            let detail = String::from_utf8_lossy(&bytes);
            return Err(format!("Hubcap returned a non-binary {description}: {}", detail.chars().take(240).collect::<String>()));
        }
        Ok(bytes)
    }

    /// Generate one exact `<depot_id>_<manifest_id>.manifest` payload.
    pub async fn generate_manifest(&self, depot_id: u32, manifest_id: u64) -> Result<Vec<u8>, String> {
        if depot_id == 0 || manifest_id == 0 {
            return Err("Hubcap generation requires a valid depot ID and manifest ID.".to_string());
        }
        let key = GenerationKey::Single { depot_id, manifest_id };
        deduplicated_generation(key, GenerationKind::Game, || async move {
            let usage = self.get_generation_usage().await?;
            self.ensure_generation_available("single-manifest", usage.single.remaining, &usage).await?;
            let url = format!(
                "{}/generate/manifest?depot_id={depot_id}&manifest_id={manifest_id}",
                BASE_URL
            );
            self.get_generated_bytes(url, "single manifest").await
        })
        .await
    }

    /// Generate a complete app bundle for a branch. The caller is responsible
    /// for extracting and installing it; this method only owns authenticated
    /// transport, quota checks, and request coalescing.
    pub async fn generate_appmanifest_bundle(&self, app_id: u32, branch: &str) -> Result<Vec<u8>, String> {
        if app_id == 0 {
            return Err("Hubcap app-manifest generation requires a valid App ID.".to_string());
        }
        let branch = if branch.trim().is_empty() { "public" } else { branch.trim() };
        let key = GenerationKey::AppBundle { app_id, branch: branch.to_string() };
        let branch_owned = branch.to_string();
        deduplicated_generation(key, GenerationKind::Game, || async move {
            let usage = self.get_generation_usage().await?;
            self.ensure_generation_available("app-bundle", usage.bundle.remaining, &usage).await?;
            let encoded_branch = encode_query_component(&branch_owned);
            let url = format!("{}/generate/appmanifest/{app_id}?branch={encoded_branch}", BASE_URL);
            self.get_generated_bytes(url, "app manifest bundle").await
        })
        .await
    }

    /// Generate one Workshop item manifest. The returned bytes are deliberately
    /// not installed here because Workshop storage is owned by Steam and the
    /// caller must supply the item/depot mapping from the originating request.
    pub async fn generate_workshop_manifest(&self, workshop_id: u64) -> Result<Vec<u8>, String> {
        if workshop_id == 0 {
            return Err("Hubcap Workshop generation requires a valid Workshop ID.".to_string());
        }
        let key = GenerationKey::Workshop { workshop_id };
        deduplicated_generation(key, GenerationKind::Workshop, || async move {
            let usage = self.get_generation_usage().await?;
            self.ensure_generation_available("Workshop", usage.workshop.remaining, &usage).await?;
            let url = format!("{}/generate/workshopmanifest/{workshop_id}", BASE_URL);
            self.get_generated_bytes(url, "Workshop manifest").await
        })
        .await
    }

    /// Broad recall search: `GET /library?search=…` with a large page size.
    pub async fn search_library(&self, query: &str) -> Result<Vec<HubcapGameItem>, String> {
        let url = format!("{}/library", BASE_URL);
        let params: Vec<(&str, String)> = vec![
            ("search", query.to_string()),
            ("limit", LIBRARY_SEARCH_LIMIT.to_string()),
        ];
        let response = self.client.get(&url)
            .headers(self.headers())
            .query(&params)
            .send()
            .await
            .map_err(|e| format!("Hubcap API network error: {}", e))?;

        if Self::is_soft_failure(response.status()) {
            eprintln!("[Hubcap] /library soft-failed with {} for '{}'", response.status(), query);
            return Ok(Vec::new());
        }

        if !response.status().is_success() {
            return Ok(Vec::new());
        }

        let data = response.json::<HubcapLibraryResponse>().await
            .map_err(|e| format!("Failed to parse Hubcap /library response: {}", e))?;

        Ok(data.games)
    }

    /// Precise search: `GET /search?q=…`. Second, independent chance to find a
    /// game when `/library`'s matcher misses it (and vice versa).
    ///
    /// Numeric queries are flagged with `appid=true` so Hubcap matches them
    /// against app ids directly (mirrors SFF's `search_by_appid` switch): this
    /// is what makes "search by App ID" light up the AVAILABLE badge too.
    pub async fn search_games(&self, query: &str) -> Result<Vec<HubcapGameItem>, String> {
        let url = format!("{}/search", BASE_URL);
        let mut params: Vec<(&str, String)> = vec![
            ("q", query.to_string()),
            ("limit", CATALOG_SEARCH_LIMIT.to_string()),
        ];
        if query.trim().parse::<u32>().is_ok() {
            params.push(("appid", "true".to_string()));
        }

        let response = self.client.get(&url)
            .headers(self.headers())
            .query(&params)
            .send()
            .await
            .map_err(|e| format!("Hubcap API network error: {}", e))?;

        if Self::is_soft_failure(response.status()) {
            eprintln!("[Hubcap] /search soft-failed with {} for '{}'", response.status(), query);
            return Ok(Vec::new());
        }

        if !response.status().is_success() {
            return Ok(Vec::new());
        }

        let data = response.json::<HubcapSearchResponse>().await
            .map_err(|e| format!("Failed to parse Hubcap /search response: {}", e))?;

        Ok(data.into_items())
    }

    /// One logical search against Hubcap, backed by two requests with different
    /// endpoints issued in parallel (`/library` + `/search`). Results are merged
    /// and de-duplicated by app id, `/library` hits first.
    ///
    /// Partial failure is tolerated: if one endpoint errors out, the other's
    /// results are still returned. Only a double failure yields an empty vec,
    /// so the caller always gets the best-effort availability set.
    pub async fn search_all(&self, query: &str) -> Vec<HubcapGameItem> {
        let (library_result, games_result) = tokio::join!(
            self.search_library(query),
            self.search_games(query),
        );

        let mut merged: Vec<HubcapGameItem> = Vec::new();
        let mut seen_ids: HashSet<u32> = HashSet::new();

        for result in [library_result, games_result] {
            match result {
                Ok(items) => {
                    for item in items {
                        if item.app_id != 0 && seen_ids.insert(item.app_id) {
                            merged.push(item);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[Hubcap] endpoint error for '{}': {}", query, e);
                }
            }
        }

        merged
    }
}
