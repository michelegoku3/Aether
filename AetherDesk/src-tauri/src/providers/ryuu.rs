use reqwest::header::{HeaderMap, HeaderValue};
use crate::manifest::package::{ManifestPackage, ManifestPackageExtractor};
use crate::providers::http;

const BASE_URL: &str = "https://generator.ryuu.lol";
const RYUU_TIMEOUT_SECONDS: u64 = 15;

/// Client for `generator.ryuu.lol` — same shape as Hubcap/Oureveryday
/// but with `X-Auth-Key` header and a single download endpoint.
///
/// All Hubcap `has_manifest` / `search` logic stays Hubcap-only;
/// Ryuu is *download-only* (no library/search, no available-badge).
/// Daily limit is 50 (enforced server-side, no stats endpoint).
#[derive(Clone)]
pub struct RyuuClient {
    api_key: String,
    client: reqwest::Client,
}

impl RyuuClient {
    pub fn new(api_key: String) -> Self {
        let mut headers = HeaderMap::new();
        // Ryuu accepts `X-Auth-Key` header (also `auth_key` query, but header is cleaner).
        if let Ok(v) = HeaderValue::from_str(&api_key) {
            headers.insert("X-Auth-Key", v);
        }
        Self {
            api_key,
            client: http::build_client_with_headers(RYUU_TIMEOUT_SECONDS, headers),
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Ok(v) = HeaderValue::from_str(&self.api_key) {
            h.insert("X-Auth-Key", v);
        }
        h
    }

    /// Downloads Ryuu's ZIP for `app_id` and extracts it as `ManifestPackage`.
    /// Uses `GET /api/download/{appid}` (no `file_type` → ZIP). The ZIP
    /// layout is the same as Hubcap's: a Lua file + optional `.manifest` files,
    /// so we can reuse `ManifestPackageExtractor`.
    pub async fn download_lua_package(&self, app_id: u32) -> Result<ManifestPackage, String> {
        let bytes = self.download_zip(app_id).await?;
        ManifestPackageExtractor::from_zip(app_id, bytes.as_ref())
    }

    async fn download_zip(&self, app_id: u32) -> Result<Vec<u8>, String> {
        crate::desk_log_info!("ryuu", "Requesting Ryuu manifest ZIP for {} from {}", crate::core::logger::format_appid(app_id), BASE_URL);
        let url = format!("{}/api/download/{}", BASE_URL, app_id);
        // Ryuu also supports `?auth_key=` query as fallback, but header is enough.
        // We keep the request minimal: just the path, auth via header.
        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| {
                crate::desk_log_error!("ryuu", "Network error requesting Ryuu manifest ZIP for {}: {}", crate::core::logger::format_appid(app_id), e);
                format!("Ryuu network error: {}", e)
            })?;

        if !resp.status().is_success() {
            crate::desk_log_error!("ryuu", "Ryuu manifest ZIP request for {} failed with HTTP status {}", crate::core::logger::format_appid(app_id), resp.status());
            return Err(format!(
                "Ryuu returned HTTP {} for App ID {}",
                resp.status(),
                app_id
            ));
        }

        let bytes = resp
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| {
                crate::desk_log_error!("ryuu", "Failed to read Ryuu manifest ZIP bytes for {}: {}", crate::core::logger::format_appid(app_id), e);
                format!("Failed to read Ryuu ZIP: {}", e)
            })?;
        crate::desk_log_info!("ryuu", "Downloaded Ryuu manifest ZIP for {} successfully ({} bytes)", crate::core::logger::format_appid(app_id), bytes.len());
        Ok(bytes)
    }
}
