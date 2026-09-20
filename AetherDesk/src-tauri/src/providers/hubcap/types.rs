use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

pub const BASE_URL: &str = "https://hubcapmanifest.com/api/v1";

/// Client-wide timeout for metadata queries (key validation, catalog search, usage).
pub const HUBCAP_TIMEOUT_SECONDS: u64 = 8;
#[allow(dead_code)]
pub const HUBCAP_METADATA_TIMEOUT_SECONDS: u64 = HUBCAP_TIMEOUT_SECONDS;

/// Timeout for downloading stored packages (ZIP archives).
pub const HUBCAP_PACKAGE_TIMEOUT_SECONDS: u64 = 30;

/// Per-request timeout for `generate/*` endpoints, applied on the request
/// builder so it replaces (not adds to) the client default. Generous on
/// purpose: a multi-hundred-MB manifest over a slow link is legitimate, while
/// a hung connection still fails in finite time.
pub const GENERATION_TIMEOUT_SECONDS: u64 = 300;
pub const GENERATION_RETRY_ATTEMPTS: u32 = 3;

/// One provider validation round-trip per key per window, shared by every
/// command surface (store, versioning, library, Workshop, settings).
pub const KEY_VALIDATION_TTL: Duration = Duration::from_secs(300);

pub const LIBRARY_SEARCH_LIMIT: u32 = 100;
pub const CATALOG_SEARCH_LIMIT: u32 = 50;

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
/// has shifted over time, and a single missing key must never turn the whole
/// search into "no results".
#[derive(Debug, Deserialize)]
pub(crate) struct HubcapLibraryResponse {
    #[serde(default)]
    pub games: Vec<HubcapGameItem>,
}

/// Envelope of `GET /search`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum HubcapSearchResponse {
    WrappedResults { results: Vec<HubcapGameItem> },
    WrappedGames { games: Vec<HubcapGameItem> },
    Bare(Vec<HubcapGameItem>),
}

impl HubcapSearchResponse {
    pub(crate) fn into_items(self) -> Vec<HubcapGameItem> {
        match self {
            Self::WrappedResults { results } => results,
            Self::WrappedGames { games } => games,
            Self::Bare(items) => items,
        }
    }
}

pub(crate) fn deserialize_app_id<'de, D>(deserializer: D) -> Result<u32, D::Error>
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

/// What Hubcap currently stores for one app, from the FREE
/// `/manifest/{app_id}/contents` endpoint.
#[derive(Debug, Clone, Default)]
pub struct HubcapAppContents {
    pub zip_exists: bool,
    /// depot_id -> manifest GID in string form
    pub manifests: HashMap<u32, String>,
    pub last_modified: Option<String>,
}

pub(crate) fn retry_delay(response: &reqwest::Response, attempt: u32) -> Duration {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| Duration::from_secs(seconds.clamp(1, 30)))
        .unwrap_or_else(|| Duration::from_secs(1u64 << attempt.min(4)))
}

pub(crate) fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status.as_u16() == 408
        || status.as_u16() == 425
        || status.as_u16() == 429
        || status.is_server_error()
}

pub(crate) fn http_failure_class(status: reqwest::StatusCode) -> &'static str {
    match status.as_u16() {
        401 | 403 => "authentication_or_authorization",
        408 | 425 | 429 => "rate_limit_or_transient_client",
        500..=599 => "provider_unavailable",
        _ => "http_error",
    }
}
