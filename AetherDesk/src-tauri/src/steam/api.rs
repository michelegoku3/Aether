use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const STEAM_APPDETAILS_URL: &str = "https://store.steampowered.com/api/appdetails";

/// Prefix used for appdetails errors caused by Steam's per-IP rate limit
/// (HTTP 429, ~200 requests/5 min on this endpoint). Callers doing bulk
/// enrichment match on this prefix to trip a circuit breaker instead of
/// continuing to hammer the endpoint.
pub const RATE_LIMIT_ERROR_PREFIX: &str = "RATE_LIMITED";

/// Shared response envelope for Steam's `appdetails` endpoint.
///
/// Used by both the DRM detector and the app-name resolver — they query the
/// same endpoint but extract different fields from `data`.
#[derive(Debug, Deserialize)]
pub struct AppDetailsEnvelope {
    pub success: bool,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// Fetch appdetails for a single app ID. Returns the parsed envelope on success.
pub async fn fetch_app_details(
    client: &reqwest::Client,
    app_id: u32,
) -> Result<AppDetailsEnvelope, String> {
    fetch_app_details_for_country(client, app_id, None).await
}

pub async fn fetch_app_details_for_country(
    client: &reqwest::Client,
    app_id: u32,
    country_code: Option<&str>,
) -> Result<AppDetailsEnvelope, String> {
    let mut params = vec![("appids", app_id.to_string()), ("l", "english".to_string())];
    if let Some(country_code) = country_code {
        params.push(("cc", country_code.trim().to_uppercase()));
    }

    let response = client
        .get(STEAM_APPDETAILS_URL)
        .query(&params)
        .send()
        .await
        .map_err(|e| format!("Steam appdetails request failed for {}: {}", app_id, e))?;

    if !response.status().is_success() {
        if response.status().as_u16() == 429 {
            return Err(format!("{}: HTTP 429 for {}", RATE_LIMIT_ERROR_PREFIX, app_id));
        }
        return Err(format!(
            "Steam appdetails returned HTTP {} for {}",
            response.status(),
            app_id
        ));
    }

    let mut data = response
        .json::<HashMap<String, AppDetailsEnvelope>>()
        .await
        .map_err(|e| format!("Failed to parse Steam appdetails for {}: {}", app_id, e))?;

    let envelope = data
        .remove(&app_id.to_string())
        .ok_or_else(|| format!("Steam appdetails did not include data for {}", app_id))?;

    if !envelope.success {
        return Err(format!("Steam appdetails returned success=false for {}", app_id));
    }

    Ok(envelope)
}

/// Run a bounded-concurrent async task for each app ID and collect results.
///
/// Steam's appdetails endpoint does not reliably accept comma-separated IDs,
/// so both the DRM detector and the name resolver use bounded single-app
/// requests. This helper centralises the Semaphore + JoinSet boilerplate.
pub async fn concurrent_app_tasks<T, F, Fut>(
    app_ids: Vec<u32>,
    concurrency: usize,
    task_fn: F,
) -> HashMap<u32, T>
where
    T: Send + 'static,
    F: Fn(u32) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Option<(u32, T)>> + Send + 'static,
{
    let mut unique_ids = app_ids;
    unique_ids.sort_unstable();
    unique_ids.dedup();

    if unique_ids.is_empty() {
        return HashMap::new();
    }

    let semaphore = Arc::new(Semaphore::new(concurrency));
    let task_fn = Arc::new(task_fn);
    let mut tasks = JoinSet::new();

    for app_id in unique_ids {
        let semaphore = Arc::clone(&semaphore);
        let task_fn = Arc::clone(&task_fn);

        tasks.spawn(async move {
            let _permit = semaphore.acquire_owned().await.ok();
            task_fn(app_id).await
        });
    }

    let mut result = HashMap::new();
    while let Some(join_result) = tasks.join_next().await {
        if let Ok(Some((app_id, value))) = join_result {
            result.insert(app_id, value);
        }
    }

    result
}
