use std::time::{Duration, Instant};

use crate::providers::hubcap_generation::{
    deduplicated_generation, GenerationKey, GenerationKind, QuotaPolicy,
};

use super::client::HubcapClient;
use super::types::{
    http_failure_class, is_retryable_status, retry_delay, BASE_URL,
    GENERATION_RETRY_ATTEMPTS, GENERATION_TIMEOUT_SECONDS, HubcapGenerationUsage,
};

impl HubcapClient {
    pub(crate) async fn ensure_generation_available(
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

    pub(crate) async fn get_generated_bytes(&self, url: String, description: &str) -> Result<Vec<u8>, String> {
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
                .timeout(Duration::from_secs(GENERATION_TIMEOUT_SECONDS))
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) if error.is_timeout() => {
                    crate::desk_log_error!(
                        "hubcap",
                        "Generation request timed out kind={} timeout_s={} elapsed_ms={} attempt={}/{}",
                        description,
                        GENERATION_TIMEOUT_SECONDS,
                        started.elapsed().as_millis(),
                        attempt + 1,
                        GENERATION_RETRY_ATTEMPTS
                    );
                    return Err(format!(
                        "Hubcap {description} request timed out after {}s (timeout_s={})",
                        started.elapsed().as_secs(),
                        GENERATION_TIMEOUT_SECONDS
                    ));
                }
                Err(error) if attempt + 1 < GENERATION_RETRY_ATTEMPTS => {
                    crate::desk_log_debug!(
                        "hubcap",
                        "Transient Hubcap generation network error; retry {}/{}: {}",
                        attempt + 2,
                        GENERATION_RETRY_ATTEMPTS,
                        error
                    );
                    tokio::time::sleep(Duration::from_millis(500 * (1u64 << attempt))).await;
                    continue;
                }
                Err(error) => {
                    crate::desk_log_error!(
                        "hubcap",
                        "Generation request network failure kind={} elapsed_ms={} after {} attempt(s): {}",
                        description,
                        started.elapsed().as_millis(),
                        attempt + 1,
                        error
                    );
                    return Err(format!(
                        "Hubcap {description} request failed after {}s: {error}",
                        started.elapsed().as_secs()
                    ));
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

            let declared_len = response
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            let bytes = response
                .bytes()
                .await
                .map_err(|error| {
                    crate::desk_log_error!(
                        "hubcap",
                        "Generation response body read failed kind={} elapsed_ms={} content_length={:?} timeout_s={} is_timeout={}: {}",
                        description,
                        started.elapsed().as_millis(),
                        declared_len,
                        GENERATION_TIMEOUT_SECONDS,
                        error.is_timeout(),
                        error
                    );
                    format!(
                        "Failed to read Hubcap {description} bytes after {}s (content-length {:?}, timeout_s={}): {error}",
                        started.elapsed().as_secs(),
                        declared_len,
                        GENERATION_TIMEOUT_SECONDS
                    )
                })?
                .to_vec();
            if bytes.is_empty() {
                crate::desk_log_error!("hubcap", "Generation response empty kind={}", description);
                return Err(format!("Hubcap returned an empty {description}."));
            }
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
                "Generation request complete kind={} bytes={} declared_bytes={:?} elapsed_ms={}",
                description,
                bytes.len(),
                declared_len,
                started.elapsed().as_millis()
            );
            if let Some(declared) = declared_len {
                if declared != bytes.len() as u64 {
                    crate::desk_log_warn!(
                        "hubcap",
                        "Generation response size mismatch kind={} declared={} received={}",
                        description,
                        declared,
                        bytes.len()
                    );
                }
            }
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
        deduplicated_generation(key, QuotaPolicy::Generate(GenerationKind::Game), || async move {
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

    /// Generate one Workshop item manifest.
    pub async fn generate_workshop_manifest(&self, workshop_id: u64) -> Result<Vec<u8>, String> {
        if workshop_id == 0 {
            return Err("Hubcap Workshop generation requires a valid Workshop ID.".to_string());
        }
        let key = GenerationKey::Workshop { workshop_id };
        deduplicated_generation(key, QuotaPolicy::Generate(GenerationKind::Workshop), || async move {
            let usage = self.get_generation_usage().await?;
            self.ensure_generation_available("Workshop", usage.workshop.remaining, &usage).await?;
            let url = format!("{}/generate/workshopmanifest/{workshop_id}", BASE_URL);
            self.get_generated_bytes(url, "Workshop manifest").await
        })
        .await
    }
}
