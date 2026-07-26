use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const STEAM_APPDETAILS_URL: &str = "https://store.steampowered.com/api/appdetails";
const DENUVO_MARKER: &str = "denuvo";
const DRM_TIMEOUT_SECONDS: u64 = 4;
const DRM_CONCURRENCY_LIMIT: usize = 6;

#[derive(Debug, Deserialize)]
struct AppDetailsEnvelope {
    success: bool,
    #[serde(default)]
    data: Option<serde_json::Value>,
}

/// Focused client for Steam Store DRM metadata.
///
/// Search results must appear fast, so DRM detection is intentionally not part of
/// StoreService::search_store. The frontend asks for Denuvo enrichment after the
/// first results are already rendered.
///
/// Steam's appdetails endpoint does not reliably accept comma-separated app IDs,
/// so `detect_many` performs bounded concurrent single-app requests instead of a
/// fake batch request. This keeps Denuvo detection correct without blocking first paint.
pub struct DrmDetector {
    client: reqwest::Client,
}

impl DrmDetector {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(DRM_TIMEOUT_SECONDS))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Returns app_id -> has_denuvo for the requested IDs.
    ///
    /// Failures for individual games are treated as `false` so enrichment never
    /// breaks the already-rendered search results.
    pub async fn detect_many(&self, app_ids: Vec<u32>) -> Result<HashMap<u32, bool>, String> {
        let mut unique_ids = app_ids;
        unique_ids.sort_unstable();
        unique_ids.dedup();

        if unique_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let semaphore = Arc::new(Semaphore::new(DRM_CONCURRENCY_LIMIT));
        let mut tasks = JoinSet::new();

        for app_id in unique_ids {
            let client = self.client.clone();
            let semaphore = Arc::clone(&semaphore);

            tasks.spawn(async move {
                let _permit = semaphore.acquire_owned().await.ok();
                let has_denuvo = Self::fetch_has_denuvo(client, app_id).await.unwrap_or(false);
                (app_id, has_denuvo)
            });
        }

        let mut result = HashMap::new();
        while let Some(join_result) = tasks.join_next().await {
            if let Ok((app_id, has_denuvo)) = join_result {
                result.insert(app_id, has_denuvo);
            }
        }

        Ok(result)
    }

    async fn fetch_has_denuvo(client: reqwest::Client, app_id: u32) -> Result<bool, String> {
        let response = client
            .get(STEAM_APPDETAILS_URL)
            .query(&[("appids", app_id.to_string()), ("l", "english".to_string())])
            .send()
            .await
            .map_err(|e| format!("Steam DRM check network error for {}: {}", app_id, e))?;

        if !response.status().is_success() {
            return Err(format!(
                "Steam DRM check returned HTTP error for {}: {}",
                app_id,
                response.status()
            ));
        }

        let data = response
            .json::<HashMap<String, AppDetailsEnvelope>>()
            .await
            .map_err(|e| format!("Failed to parse Steam DRM response for {}: {}", app_id, e))?;

        let has_denuvo = data
            .get(&app_id.to_string())
            .filter(|envelope| envelope.success)
            .and_then(|envelope| envelope.data.as_ref())
            .and_then(|details| details.get("drm_notice"))
            .and_then(|notice| notice.as_str())
            .map(|notice| notice.to_lowercase().contains(DENUVO_MARKER))
            .unwrap_or(false);

        Ok(has_denuvo)
    }
}
