use std::collections::HashMap;

use crate::manifest::package::{ManifestPackage, ManifestPackageExtractor};
use crate::providers::http;
use crate::providers::luatools_auth::LuaToolsAuth;

const API_BASE_URL: &str = "https://lua.tools";
const DOWNLOAD_TIMEOUT_SECONDS: u64 = 5 * 60;

/// LuaTools is an authenticated source aggregator. Availability is discovered
/// from its manifest backend; downloads use the user's own LuaTools Supabase
/// session and therefore count against that account's server-side allowance.
pub struct LuaToolsClient {
    auth: LuaToolsAuth,
    client: reqwest::Client,
}

impl LuaToolsClient {
    pub fn new() -> Self {
        Self {
            auth: LuaToolsAuth::new(),
            client: http::build_client(DOWNLOAD_TIMEOUT_SECONDS),
        }
    }

    pub async fn download_lua_package(&self, app_id: u32) -> Result<ManifestPackage, String> {
        let access_token = self.auth.valid_access_token().await?;
        let source = self.select_source(app_id, &access_token).await?;
        let response = self
            .client
            .get(format!("{API_BASE_URL}/api/manifest/download"))
            .bearer_auth(access_token)
            .query(&[
                ("appid", app_id.to_string()),
                ("source", source.clone()),
            ])
            .send()
            .await
            .map_err(|e| format!("LuaTools download failed: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            return Err(format!(
                "LuaTools source {source} returned HTTP {status}: {detail}"
            ));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("Could not read the LuaTools download: {e}"))?;
        crate::desk_log_info!(
            "luatools",
            "Downloaded LuaTools package for {} from source '{}' ({} bytes, content-type={})",
            crate::core::logger::format_appid(app_id),
            source,
            bytes.len(),
            content_type
        );
        ManifestPackageExtractor::from_provider_bytes(app_id, bytes.as_ref())
    }

    async fn select_source(&self, app_id: u32, access_token: &str) -> Result<String, String> {
        let response = self
            .client
            .get(format!("{API_BASE_URL}/api/manifest/check"))
            .bearer_auth(access_token)
            .query(&[("appid", app_id)])
            .send()
            .await
            .map_err(|e| format!("Could not check LuaTools sources: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "LuaTools source check returned HTTP {}",
                response.status()
            ));
        }
        let statuses: HashMap<String, String> = response
            .json()
            .await
            .map_err(|e| format!("Could not parse LuaTools source availability: {e}"))?;

        choose_available_source(&statuses)
            .ok_or_else(|| format!("LuaTools has no available source for App ID {app_id}"))
    }
}

/// Stable preference keeps automatic selection deterministic even though JSON
/// object order is not part of the manifest backend contract.
pub(crate) fn choose_available_source(statuses: &HashMap<String, String>) -> Option<String> {
    const PREFERENCE: &[&str] = &["Luie", "Ryuu", "Sushi", "Skyflare", "TwentyTwo Cloud"];
    for preferred in PREFERENCE {
        if let Some((name, _)) = statuses.iter().find(|(name, status)| {
            name.eq_ignore_ascii_case(preferred) && status.eq_ignore_ascii_case("available")
        }) {
            return Some(name.clone());
        }
    }
    statuses
        .iter()
        .filter(|(_, status)| status.eq_ignore_ascii_case("available"))
        .map(|(name, _)| name.clone())
        .min()
}
