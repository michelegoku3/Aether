use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::providers::http;
use crate::steam::api::{self, AppDetailsEnvelope};

const DRM_TIMEOUT_SECONDS: u64 = 4;
/// Kept deliberately low: appdetails is rate-limited per IP (~200 req/5 min),
/// so burst behaviour is the enemy, not slowness.
const DRM_CONCURRENCY_LIMIT: usize = 3;
const DENUVO_MARKER: &str = "denuvo";
const CACHE_FILE_NAME: &str = "denuvo_cache.json";
/// Denuvo status changes rarely (patches removing it months after release),
/// so a 30-day TTL keeps the cache trustworthy while making every app id a
/// network call at most once a month.
const CACHE_TTL_SECONDS: u64 = 30 * 24 * 60 * 60;

#[derive(Debug, Default, Serialize, Deserialize)]
struct DenuvoCacheFile {
    /// Same self-describing invalidation scheme as the store search cache:
    /// entries written by a different AetherDesk build are ignored, no stamp file.
    #[serde(default)]
    app_version: String,
    entries: HashMap<u32, DenuvoCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DenuvoCacheEntry {
    has_denuvo: bool,
    checked_at_unix: u64,
}

/// Focused client for Steam Store DRM metadata.
///
/// Search results must appear fast, so DRM detection is intentionally not part of
/// StoreService::search_store. The frontend asks for Denuvo enrichment after the
/// first results are already rendered — and only for the currently visible page.
///
/// Rate-limit safety, three layers:
///   1. persistent 30-day disk cache (repeats are free);
///   2. bounded concurrency (3) on the rate-limited appdetails endpoint;
///   3. circuit breaker on HTTP 429: the first rate-limited response aborts
///      every pending fetch instead of making Steam more angry.
/// Failed fetches are simply absent from the result map (never cached as
/// `false`), so a transient outage never poisons future lookups.
pub struct DrmDetector {
    client: reqwest::Client,
    cache_path: PathBuf,
    /// Version of the running build (from `tauri.conf.json`), for
    /// self-describing cache invalidation.
    app_version: String,
}

impl DrmDetector {
    pub fn new(cache_dir: PathBuf, app_version: String) -> Self {
        Self {
            client: http::build_client(DRM_TIMEOUT_SECONDS),
            cache_path: cache_dir.join(CACHE_FILE_NAME),
            app_version,
        }
    }

    /// Returns app_id -> has_denuvo for the requested IDs.
    pub async fn detect_many(&self, app_ids: Vec<u32>) -> Result<HashMap<u32, bool>, String> {
        let mut cache = self.load_cache();
        let now = Self::now_unix();

        // 1. Serve fresh cache hits without touching the network.
        let mut results: HashMap<u32, bool> = HashMap::new();
        let mut missing: Vec<u32> = Vec::new();
        for app_id in app_ids {
            match cache.entries.get(&app_id) {
                Some(entry) if now.saturating_sub(entry.checked_at_unix) <= CACHE_TTL_SECONDS => {
                    results.insert(app_id, entry.has_denuvo);
                }
                _ => missing.push(app_id),
            }
        }

        if missing.is_empty() {
            return Ok(results);
        }

        // 2. Fetch the unknown ones, bounded + circuit-broken.
        // The shared flag is cloned into the task factory below; the original
        // `rate_limited` stays owned by this scope so we can inspect it after
        // the join (an Arc moved into a `move` closure is gone for the caller).
        let rate_limited = Arc::new(AtomicBool::new(false));
        let client = self.client.clone();
        let fetched = api::concurrent_app_tasks(
            missing,
            DRM_CONCURRENCY_LIMIT,
            {
                let rate_limited = Arc::clone(&rate_limited);
                move |app_id| {
                    let client = client.clone();
                    let breaker = Arc::clone(&rate_limited);
                    async move {
                        // Breaker already tripped: skip silently instead of queueing
                        // behind the semaphore just to be refused by Steam.
                        if breaker.load(Ordering::Relaxed) {
                            return None;
                        }
                        match Self::fetch_has_denuvo(&client, app_id).await {
                            Ok(has_denuvo) => Some((app_id, has_denuvo)),
                            Err(e) => {
                                if e.starts_with(api::RATE_LIMIT_ERROR_PREFIX) {
                                    breaker.store(true, Ordering::Relaxed);
                                    eprintln!(
                                        "[DRM] Steam rate limit hit; aborting remaining Denuvo checks"
                                    );
                                } else {
                                    eprintln!("[DRM] Denuvo check failed for {}: {}", app_id, e);
                                }
                                None // transient failures are not cached
                            }
                        }
                    }
                }
            },
        )
        .await;
        if rate_limited.load(Ordering::Relaxed) {
            eprintln!("[DRM] Denuvo enrichment partially served from cache ({} fresh hits)", results.len());
        }

        // 3. Persist only what Steam actually answered.
        if !fetched.is_empty() {
            let now = Self::now_unix();
            for (app_id, has_denuvo) in &fetched {
                cache.entries.insert(
                    *app_id,
                    DenuvoCacheEntry {
                        has_denuvo: *has_denuvo,
                        checked_at_unix: now,
                    },
                );
            }
            if let Err(e) = self.save_cache(&cache) {
                eprintln!("[DRM] Failed to persist Denuvo cache: {}", e);
            }
        }
        results.extend(fetched);

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

    fn load_cache(&self) -> DenuvoCacheFile {
        let Ok(content) = fs::read_to_string(&self.cache_path) else {
            return DenuvoCacheFile::default();
        };
        let mut cache = serde_json::from_str::<DenuvoCacheFile>(&content).unwrap_or_default();
        if cache.app_version != self.app_version {
            cache.entries.clear();
        }
        cache
    }

    fn save_cache(&self, cache: &DenuvoCacheFile) -> Result<(), String> {
        if let Some(parent) = self.cache_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create Denuvo cache directory: {}", e))?;
        }
        let temp_path = self.cache_path.with_extension("tmp");
        let stamped = DenuvoCacheFile {
            app_version: self.app_version.clone(),
            entries: cache.entries.clone(),
        };
        let content = serde_json::to_string(&stamped)
            .map_err(|e| format!("Failed to serialize Denuvo cache: {}", e))?;
        fs::write(&temp_path, &content)
            .map_err(|e| format!("Failed to write Denuvo cache: {}", e))?;
        fs::rename(&temp_path, &self.cache_path)
            .map_err(|e| format!("Failed to apply Denuvo cache: {}", e))
    }

    fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default()
    }
}
