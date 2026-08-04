use std::collections::HashMap;
use crate::providers::http;
use crate::steam::api::{self, AppDetailsEnvelope};

const DRM_TIMEOUT_SECONDS: u64 = 4;
const DRM_CONCURRENCY_LIMIT: usize = 6;
const DENUVO_MARKER: &str = "denuvo";

/// Focused client for Steam Store DRM metadata.
///
/// Search results must appear fast, so DRM detection is intentionally not part of
/// StoreService::search_store. The frontend asks for Denuvo enrichment after the
/// first results are already rendered.
pub struct DrmDetector {
    client: reqwest::Client,
}

impl DrmDetector {
    pub fn new() -> Self {
        Self {
            client: http::build_client(DRM_TIMEOUT_SECONDS),
        }
    }

    /// Returns app_id -> has_denuvo for the requested IDs.
    ///
    /// Failures for individual games are treated as `false` so enrichment never
    /// breaks the already-rendered search results.
    pub async fn detect_many(&self, app_ids: Vec<u32>) -> Result<HashMap<u32, bool>, String> {
        let client = self.client.clone();
        let results = api::concurrent_app_tasks(
            app_ids,
            DRM_CONCURRENCY_LIMIT,
            move |app_id| {
                let client = client.clone();
                async move {
                    let has_denuvo = Self::fetch_has_denuvo(&client, app_id).await.unwrap_or(false);
                    Some((app_id, has_denuvo))
                }
            },
        )
        .await;

        Ok(results)
    }

    async fn fetch_has_denuvo(client: &reqwest::Client, app_id: u32) -> Result<bool, String> {
        let envelope: AppDetailsEnvelope = api::fetch_app_details(client, app_id).await?;

        let has_denuvo = envelope
            .data
            .as_ref()
            .and_then(|details| details.get("drm_notice"))
            .and_then(|notice| notice.as_str())
            .map(|notice| notice.to_lowercase().contains(DENUVO_MARKER))
            .unwrap_or(false);

        Ok(has_denuvo)
    }
}
