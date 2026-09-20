use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;

use super::client::HubcapClient;
use super::types::{
    http_failure_class, is_retryable_status, retry_delay, BASE_URL,
    GENERATION_RETRY_ATTEMPTS, HubcapGenerationUsage, HubcapUserStats, KEY_VALIDATION_TTL,
};

/// Session-wide validation cache. The fingerprint is the SHA-256 of the raw
/// key, so a changed key is always re-validated immediately.
fn validation_cache() -> &'static AsyncMutex<Option<(Vec<u8>, Instant, bool)>> {
    static CACHE: OnceLock<AsyncMutex<Option<(Vec<u8>, Instant, bool)>>> = OnceLock::new();
    CACHE.get_or_init(|| AsyncMutex::new(None))
}

impl HubcapClient {
    /// Validates the API key against `/user/stats`, deduplicated across the
    /// whole session by a short-lived fingerprint cache.
    pub async fn validate_api_key(&self) -> Result<bool, String> {
        let fingerprint = Sha256::digest(self.api_key.as_bytes()).to_vec();
        {
            let cached = validation_cache().lock().await;
            if let Some((cached_fingerprint, checked_at, valid)) = cached.as_ref() {
                if *cached_fingerprint == fingerprint && checked_at.elapsed() < KEY_VALIDATION_TTL {
                    crate::desk_log_debug!("hubcap", "API key validation served from session cache");
                    return Ok(*valid);
                }
            }
        }
        let valid = self.validate_api_key_uncached().await?;
        *validation_cache().lock().await = Some((fingerprint, Instant::now(), valid));
        Ok(valid)
    }

    async fn validate_api_key_uncached(&self) -> Result<bool, String> {
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
                    crate::desk_log_error!(
                        "hubcap",
                        "API key validation network failure after {} attempt(s): {}",
                        GENERATION_RETRY_ATTEMPTS,
                        error
                    );
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

    /// Returns the server-side generation quota and Steam readiness.
    pub async fn get_generation_usage(&self) -> Result<HubcapGenerationUsage, String> {
        let url = format!("{}/generate/usage", BASE_URL);
        for attempt in 0..GENERATION_RETRY_ATTEMPTS {
            let response = match self.client.get(&url).headers(self.headers()).send().await {
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
}
