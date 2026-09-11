use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use crate::manifest::package::{ManifestPackage, ManifestPackageExtractor};
use crate::providers::http;
use crate::providers::hubcap_generation::{
    deduplicated_generation, GenerationKey, GenerationKind,
};

const BASE_URL: &str = "https://hubcapmanifest.com/api/v1";
const HUBCAP_TIMEOUT_SECONDS: u64 = 8;
const GENERATION_RETRY_ATTEMPTS: u32 = 3;

fn retry_delay(response: &reqwest::Response, attempt: u32) -> Duration {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| Duration::from_secs(seconds.clamp(1, 30)))
        .unwrap_or_else(|| Duration::from_secs(1u64 << attempt.min(4)))
}

fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status.as_u16() == 408
        || status.as_u16() == 425
        || status.as_u16() == 429
        || status.is_server_error()
}

fn http_failure_class(status: reqwest::StatusCode) -> &'static str {
    match status.as_u16() {
        401 | 403 => "authentication_or_authorization",
        408 | 425 | 429 => "rate_limit_or_transient_client",
        500..=599 => "provider_unavailable",
        _ => "http_error",
    }
}

/// `/library` gets a moderate page: bigger pages mean bigger payloads and a
/// longer Hubcap-only tail that the pre-filter then has to cut down anyway.
const LIBRARY_SEARCH_LIMIT: u32 = 100;
const CATALOG_SEARCH_LIMIT: u32 = 50;

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
        crate::desk_log_info!("hubcap", "API key validation start endpoint=user/stats");
        let url = format!("{}/user/stats", BASE_URL);
        for attempt in 0..GENERATION_RETRY_ATTEMPTS {
            let response = match self.client.get(&url).headers(self.headers()).send().await {
                Ok(response) => response,
                Err(error) if attempt + 1 < GENERATION_RETRY_ATTEMPTS => {
                    crate::desk_log_warn!(
                        "hubcap",
                        "API key validation network error attempt={}/{}; retrying: {}",
                        attempt + 1,
                        GENERATION_RETRY_ATTEMPTS,
                        error
                    );
                    tokio::time::sleep(Duration::from_millis(500 * (1u64 << attempt))).await;
                    continue;
                }
                Err(error) => {
                    crate::desk_log_error!("hubcap", "API key validation network failure after {} attempt(s): {}", GENERATION_RETRY_ATTEMPTS, error);
                    return Err(format!("Network error: {error}"));
                }
            };
            if is_retryable_status(response.status()) && attempt + 1 < GENERATION_RETRY_ATTEMPTS {
                let status = response.status();
                crate::desk_log_warn!(
                    "hubcap",
                    "API key validation HTTP {} attempt={}/{}; retrying",
                    status,
                    attempt + 1,
                    GENERATION_RETRY_ATTEMPTS
                );
                tokio::time::sleep(retry_delay(&response, attempt)).await;
                continue;
            }
            if response.status().is_success() {
                let stats = response
                    .json::<HubcapUserStats>()
                    .await
                    .map_err(|error| format!("Failed to parse Hubcap account status: {error}"))?;
                let active = stats.can_make_requests.unwrap_or(true);
                if active {
                    crate::desk_log_info!("hubcap", "API key validation complete active=true");
                } else {
                    crate::desk_log_warn!("hubcap", "API key authenticated but account cannot make requests");
                }
                return Ok(active);
            }
            if response.status().as_u16() == 401 {
                crate::desk_log_warn!("hubcap", "API key validation complete active=false reason=unauthorized");
                return Ok(false);
            }
            crate::desk_log_error!(
                "hubcap",
                "API key validation failed class={} HTTP {}",
                http_failure_class(response.status()),
                response.status()
            );
            return Err(format!("Server returned HTTP error: {}", response.status()));
        }
        unreachable!("API key validation retry loop always returns")
    }

    /// Downloads Hubcap's manifest ZIP after an authenticated quota check and
    /// delegates archive parsing to the provider-agnostic `ManifestPackageExtractor`.
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
        crate::desk_log_debug!(
            "hubcap",
            "Manifest ZIP quota check app_id={} daily_usage={:?} daily_limit={:?} can_make_requests={:?}",
            app_id,
            stats.daily_usage,
            stats.role_daily_limit.or(stats.daily_limit),
            stats.can_make_requests
        );
        if stats.can_make_requests == Some(false) {
            return Err("Hubcap account is not allowed to make manifest requests.".to_string());
        }
        let limit = stats.role_daily_limit.or(stats.daily_limit);
        if let (Some(usage), Some(limit)) = (stats.daily_usage, limit) {
            if usage >= limit {
                return Err(format!("Hubcap account daily manifest quota is exhausted ({usage}/{limit})."));
            }
        }
        let url = format!("{}/manifest/{}", BASE_URL, app_id);
        let started = Instant::now();
        for attempt in 0..GENERATION_RETRY_ATTEMPTS {
            crate::desk_log_info!(
                "hubcap",
                "Manifest ZIP request start app_id={} attempt={}/{}",
                app_id,
                attempt + 1,
                GENERATION_RETRY_ATTEMPTS
            );
            let response = match self.client.get(&url).headers(self.headers()).send().await {
                Ok(response) => response,
                Err(error) if attempt + 1 < GENERATION_RETRY_ATTEMPTS => {
                    crate::desk_log_warn!(
                        "hubcap",
                        "Manifest ZIP network error app_id={} attempt={}/{}; retrying: {}",
                        app_id,
                        attempt + 1,
                        GENERATION_RETRY_ATTEMPTS,
                        error
                    );
                    tokio::time::sleep(Duration::from_millis(500 * (1u64 << attempt))).await;
                    continue;
                }
                Err(error) => {
                    crate::desk_log_error!(
                        "hubcap",
                        "Manifest ZIP network error app_id={} after {} attempt(s): {}",
                        app_id,
                        GENERATION_RETRY_ATTEMPTS,
                        error
                    );
                    return Err(format!("Failed to send manifest ZIP request: {error}"));
                }
            };
            if is_retryable_status(response.status()) && attempt + 1 < GENERATION_RETRY_ATTEMPTS {
                let status = response.status();
                crate::desk_log_warn!(
                    "hubcap",
                    "Manifest ZIP HTTP {} app_id={} attempt={}/{}; retrying",
                    status,
                    app_id,
                    attempt + 1,
                    GENERATION_RETRY_ATTEMPTS
                );
                tokio::time::sleep(retry_delay(&response, attempt)).await;
                continue;
            }
            if !response.status().is_success() {
                crate::desk_log_error!(
                    "hubcap",
                    "Manifest ZIP request failed app_id={} class={} HTTP {} after {} attempt(s)",
                    app_id,
                    http_failure_class(response.status()),
                    response.status(),
                    attempt + 1
                );
                return Err(format!(
                    "Failed to retrieve manifest ZIP ({}) HTTP Status: {}",
                    http_failure_class(response.status()),
                    response.status()
                ));
            }
            let bytes = response.bytes().await.map_err(|error| {
                crate::desk_log_error!(
                    "hubcap",
                    "Manifest ZIP body read failed app_id={} after {} attempt(s): {}",
                    app_id,
                    attempt + 1,
                    error
                );
                format!("Failed to read manifest ZIP bytes: {error}")
            })?.to_vec();
            if bytes.is_empty() {
                return Err(format!("Hubcap returned an empty manifest ZIP for AppID {app_id}."));
            }
            crate::desk_log_info!(
                "hubcap",
                "Manifest ZIP request complete app_id={} bytes={} elapsed_ms={}",
                app_id,
                bytes.len(),
                started.elapsed().as_millis()
            );
            return Ok(bytes);
        }
        unreachable!("manifest ZIP retry loop always returns")
    }

    /// Lightweight existence check: does Hubcap have a manifest for this `app_id`?
    /// Uses `GET /status/{id}` (Free - No usage count per Api Endpoints.txt),
    /// never `GET /manifest/{id}` which *counts* toward daily usage.
    /// Interprets `manifest_file_exists == true` or `status == "available"` as true.
    /// Any non-200, parse error, or network failure is `false`; callers must not
    /// treat an inconclusive status response as a verified manifest.
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
                    Err(error) => {
                        crate::desk_log_warn!(
                            "hubcap",
                            "Manifest status response parse failed app_id={}: {}",
                            app_id,
                            error
                        );
                        false
                    }
                }
            }
            Ok(response) => {
                crate::desk_log_warn!(
                    "hubcap",
                    "Manifest status check app_id={} HTTP {}",
                    app_id,
                    response.status()
                );
                false
            }
            Err(error) => {
                crate::desk_log_warn!("hubcap", "Manifest status network check failed app_id={}: {}", app_id, error);
                false
            }
        }
    }

    pub async fn get_usage_stats(&self) -> Result<HubcapUserStats, String> {
        let url = format!("{}/user/stats", BASE_URL);
        for attempt in 0..GENERATION_RETRY_ATTEMPTS {
            let response = match self.client.get(&url).headers(self.headers()).send().await {
                Ok(response) => response,
                Err(error) if attempt + 1 < GENERATION_RETRY_ATTEMPTS => {
                    crate::desk_log_warn!(
                        "hubcap",
                        "Usage stats network error attempt={}/{}; retrying: {}",
                        attempt + 1,
                        GENERATION_RETRY_ATTEMPTS,
                        error
                    );
                    tokio::time::sleep(Duration::from_millis(500 * (1u64 << attempt))).await;
                    continue;
                }
                Err(error) => return Err(format!("Network error: {error}")),
            };
            if is_retryable_status(response.status()) && attempt + 1 < GENERATION_RETRY_ATTEMPTS {
                let status = response.status();
                crate::desk_log_warn!(
                    "hubcap",
                    "Usage stats HTTP {} attempt={}/{}; retrying",
                    status,
                    attempt + 1,
                    GENERATION_RETRY_ATTEMPTS
                );
                tokio::time::sleep(retry_delay(&response, attempt)).await;
                continue;
            }
            if response.status().is_success() {
                return response
                    .json::<HubcapUserStats>()
                    .await
                    .map_err(|error| format!("Failed to parse user stats: {error}"));
            }
            return Err(format!(
                "Hubcap usage stats {} (HTTP {}): {}",
                http_failure_class(response.status()),
                response.status(),
                response.status().canonical_reason().unwrap_or("Unknown")
            ));
        }
        unreachable!("usage stats retry loop always returns")
    }

    /// Returns the server-side generation quota and Steam readiness. This is a
    /// free endpoint and is checked immediately before every generation request;
    /// the local scheduler still enforces the larger application-level caps.
    pub async fn get_generation_usage(&self) -> Result<HubcapGenerationUsage, String> {
        let url = format!("{}/generate/usage", BASE_URL);
        for attempt in 0..GENERATION_RETRY_ATTEMPTS {
            let response = match self
                .client
                .get(&url)
                .headers(self.headers())
                .send()
                .await
            {
                Ok(response) => response,
                Err(_error) if attempt + 1 < GENERATION_RETRY_ATTEMPTS => {
                    crate::desk_log_debug!(
                        "hubcap",
                        "Transient Hubcap generation network error; retry {}/{}",
                        attempt + 2,
                        GENERATION_RETRY_ATTEMPTS
                    );
                    tokio::time::sleep(Duration::from_millis(500 * (1u64 << attempt))).await;
                    continue;
                }
                Err(error) => {
                    return Err(format!("Hubcap generation-usage request failed: {error}"));
                }
            };
            let status = response.status();
            let retryable = is_retryable_status(status);
            if retryable && attempt + 1 < GENERATION_RETRY_ATTEMPTS {
                crate::desk_log_debug!(
                    "hubcap",
                    "Transient Hubcap generation-usage HTTP {}; retry {}/{}",
                    status,
                    attempt + 2,
                    GENERATION_RETRY_ATTEMPTS
                );
                tokio::time::sleep(retry_delay(&response, attempt)).await;
                continue;
            }
            if !response.status().is_success() {
                crate::desk_log_error!(
                    "hubcap",
                    "Generation usage failed class={} HTTP {}",
                    http_failure_class(response.status()),
                    response.status()
                );
                return Err(format!(
                    "Hubcap generation usage {} (HTTP {})",
                    http_failure_class(response.status()),
                    response.status()
                ));
            }
            let usage = response
                .json::<HubcapGenerationUsage>()
                .await
                .map_err(|e| format!("Failed to parse Hubcap generation usage: {e}"))?;
            crate::desk_log_debug!(
                "hubcap",
                "Generation usage received single={:?}/{:?} workshop={:?}/{:?} steam_service_ready={:?}",
                usage.single.usage,
                usage.single.limit,
                usage.workshop.usage,
                usage.workshop.limit,
                usage.steam_service_ready
            );
            return Ok(usage);
        }
        unreachable!("generation usage retry loop always returns")
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
        let started = Instant::now();
        for attempt in 0..GENERATION_RETRY_ATTEMPTS {
            crate::desk_log_debug!(
                "hubcap",
                "Generation request start kind={} attempt={}/{}",
                description,
                attempt + 1,
                GENERATION_RETRY_ATTEMPTS
            );
            let response = match self
                .client
                .get(&url)
                .headers(self.headers())
                .send()
                .await
            {
                Ok(response) => response,
                Err(_error) if attempt + 1 < GENERATION_RETRY_ATTEMPTS => {
                    crate::desk_log_debug!(
                        "hubcap",
                        "Transient Hubcap generation network error; retry {}/{}",
                        attempt + 2,
                        GENERATION_RETRY_ATTEMPTS
                    );
                    tokio::time::sleep(Duration::from_millis(500 * (1u64 << attempt))).await;
                    continue;
                }
                Err(error) => {
                    crate::desk_log_error!(
                        "hubcap",
                        "Generation request network failure kind={} after {} attempt(s): {}",
                        description,
                        attempt + 1,
                        error
                    );
                    return Err(format!("Hubcap {description} request failed: {error}"));
                }
            };
            let status = response.status();
            let retryable = is_retryable_status(status);
            if retryable && attempt + 1 < GENERATION_RETRY_ATTEMPTS {
                crate::desk_log_debug!(
                    "hubcap",
                    "Transient Hubcap generation HTTP {}; retry {}/{}",
                    status,
                    attempt + 2,
                    GENERATION_RETRY_ATTEMPTS
                );
                tokio::time::sleep(retry_delay(&response, attempt)).await;
                continue;
            }
            if !response.status().is_success() {
                crate::desk_log_error!(
                    "hubcap",
                    "Generation request failed kind={} class={} HTTP {} after {} attempt(s)",
                    description,
                    http_failure_class(response.status()),
                    response.status(),
                    attempt + 1
                );
                return Err(format!(
                    "Hubcap {description} request {} (HTTP {})",
                    http_failure_class(response.status()),
                    response.status()
                ));
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
                .map_err(|error| {
                    crate::desk_log_error!("hubcap", "Generation response body read failed kind={}: {}", description, error);
                    format!("Failed to read Hubcap {description} bytes: {error}")
                })?
                .to_vec();
            if bytes.is_empty() {
                crate::desk_log_error!("hubcap", "Generation response empty kind={}", description);
                return Err(format!("Hubcap returned an empty {description}."));
            }
            // Generation endpoints return binary data. Surface a useful provider
            // error instead of writing a JSON error document as a .manifest file.
            if content_type.contains("json") || content_type.starts_with("text/") {
                let detail = String::from_utf8_lossy(&bytes);
                crate::desk_log_error!(
                    "hubcap",
                    "Generation response non-binary kind={} content_type={} detail_len={}",
                    description,
                    content_type,
                    detail.len()
                );
                return Err(format!("Hubcap returned a non-binary {description}: {}", detail.chars().take(240).collect::<String>()));
            }
            crate::desk_log_info!(
                "hubcap",
                "Generation request complete kind={} bytes={} elapsed_ms={}",
                description,
                bytes.len(),
                started.elapsed().as_millis()
            );
            return Ok(bytes);
        }
        unreachable!("generated-byte retry loop always returns")
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
            crate::desk_log_warn!("hubcap", "Library search soft-failed HTTP {} query_len={}", response.status(), query.len());
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
            crate::desk_log_warn!("hubcap", "Catalog search soft-failed HTTP {} query_len={}", response.status(), query.len());
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
                Err(error) => {
                    crate::desk_log_warn!("hubcap", "Search endpoint failed query_len={}: {}", query.len(), error);
                }
            }
        }

        merged
    }
}
