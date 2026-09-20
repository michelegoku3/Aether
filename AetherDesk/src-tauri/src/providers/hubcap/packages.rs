use std::time::{Duration, Instant};

use crate::manifest::package::{ManifestPackage, ManifestPackageExtractor};
use crate::providers::hubcap_generation::{
    deduplicated_generation, GenerationKey, QuotaPolicy,
};

use super::client::HubcapClient;
use super::types::{
    http_failure_class, is_retryable_status, retry_delay, BASE_URL,
    GENERATION_RETRY_ATTEMPTS, HUBCAP_PACKAGE_TIMEOUT_SECONDS, HubcapAppContents,
};

impl HubcapClient {
    /// Downloads Hubcap's manifest ZIP after an authenticated quota check and
    /// delegates archive parsing to the provider-agnostic `ManifestPackageExtractor`.
    pub async fn download_lua_package(&self, app_id: u32) -> Result<ManifestPackage, String> {
        let bytes = deduplicated_generation(
            GenerationKey::AppBundle {
                app_id,
                branch: "manifest".to_string(),
            },
            QuotaPolicy::ManifestDownload,
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
            let response = match self
                .client
                .get(&url)
                .headers(self.headers())
                .timeout(Duration::from_secs(HUBCAP_PACKAGE_TIMEOUT_SECONDS))
                .send()
                .await
            {
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

    /// Lists the manifests inside Hubcap's stored package for one app.
    pub async fn get_app_contents(&self, app_id: u32) -> Result<HubcapAppContents, String> {
        fn id_to_string(value: Option<&serde_json::Value>) -> Option<String> {
            match value? {
                serde_json::Value::String(text) => {
                    let trimmed = text.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                }
                serde_json::Value::Number(number) => Some(number.to_string()),
                _ => None,
            }
        }

        let url = format!("{}/manifest/{}/contents", BASE_URL, app_id);
        let response = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|error| format!("Hubcap contents request failed for app {app_id}: {error}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "Hubcap contents check for app {app_id} returned HTTP {}",
                response.status()
            ));
        }
        let body: serde_json::Value = response.json().await.map_err(|error| {
            format!("Hubcap contents response parse failed for app {app_id}: {error}")
        })?;

        let mut contents = HubcapAppContents {
            zip_exists: body.get("zip_exists").and_then(|value| value.as_bool()).unwrap_or(false),
            last_modified: body
                .get("last_modified")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            ..Default::default()
        };
        if let Some(entries) = body.get("manifests").and_then(|value| value.as_array()) {
            for entry in entries {
                let (Some(depot), Some(gid)) = (
                    id_to_string(entry.get("depot_id")),
                    id_to_string(entry.get("manifest_id")),
                ) else {
                    continue;
                };
                let Ok(depot_id) = depot.parse::<u32>() else {
                    continue;
                };
                if depot_id == 0 || gid == "0" {
                    continue;
                }
                contents.manifests.insert(depot_id, gid);
            }
        }
        crate::desk_log_debug!(
            "hubcap",
            "Contents check app_id={} zip_exists={} manifests={} last_modified={}",
            app_id,
            contents.zip_exists,
            contents.manifests.len(),
            contents.last_modified.clone().unwrap_or_else(|| "-".to_string())
        );
        Ok(contents)
    }
}
