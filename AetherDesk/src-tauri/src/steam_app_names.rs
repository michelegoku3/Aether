use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const STEAM_APPDETAILS_URL: &str = "https://store.steampowered.com/api/appdetails";
const NAME_RESOLVE_TIMEOUT_SECONDS: u64 = 4;
const NAME_RESOLVE_CONCURRENCY: usize = 8;
const CACHE_FILE_NAME: &str = "steam_app_names.json";

#[derive(Debug, Deserialize)]
struct AppDetailsEnvelope {
    success: bool,
    #[serde(default)]
    data: Option<serde_json::Value>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NameCacheFile {
    names: HashMap<u32, String>,
}

/// Resolves Steam app names by App ID and stores them in a persistent cache.
///
/// The Library view is based on local Lua files, whose comments can sometimes contain
/// noisy/wrong titles. This resolver mirrors the Store source of truth by asking Steam
/// appdetails for the canonical app name, then caching it so later Library opens are fast.
pub struct SteamAppNameResolver {
    cache_path: PathBuf,
    client: reqwest::Client,
}

impl SteamAppNameResolver {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_path: cache_dir.join(CACHE_FILE_NAME),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(NAME_RESOLVE_TIMEOUT_SECONDS))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Returns names already present in the persistent cache without doing any network I/O.
    ///
    /// This method is intentionally synchronous and fast: it is used by the Library UI path so
    /// games can be rendered immediately even when Steam is offline or slow.
    pub fn cached_names(&self, app_ids: Vec<u32>) -> HashMap<u32, String> {
        let cache = self.load_cache();
        Self::filter_cached_names(cache, app_ids)
    }

    /// Resolves missing names through Steam and persists them for future cache-only reads.
    ///
    /// Use this from warm-up/background paths, not from UI-critical commands.
    pub async fn resolve_names(&self, app_ids: Vec<u32>) -> HashMap<u32, String> {
        let mut cache = self.load_cache();
        let unique_ids = Self::unique_app_ids(app_ids);

        let missing_ids: Vec<u32> = unique_ids
            .iter()
            .copied()
            .filter(|app_id| !cache.names.contains_key(app_id))
            .collect();

        if !missing_ids.is_empty() {
            let fetched = self.fetch_missing_names(missing_ids).await;
            let mut changed = false;

            for (app_id, name) in fetched {
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    cache.names.insert(app_id, trimmed.to_string());
                    changed = true;
                }
            }

            if changed {
                let _ = self.save_cache(&cache);
            }
        }

        Self::filter_cached_names(cache, unique_ids)
    }

    fn unique_app_ids(mut app_ids: Vec<u32>) -> Vec<u32> {
        app_ids.sort_unstable();
        app_ids.dedup();
        app_ids
    }

    fn filter_cached_names(cache: NameCacheFile, app_ids: Vec<u32>) -> HashMap<u32, String> {
        Self::unique_app_ids(app_ids)
            .into_iter()
            .filter_map(|app_id| {
                cache
                    .names
                    .get(&app_id)
                    .cloned()
                    .filter(|name| !name.trim().is_empty())
                    .map(|name| (app_id, name))
            })
            .collect()
    }


    /// Merges trusted names obtained from another local/live source (for example Store search)
    /// into the shared app-name cache used by the Library.
    pub fn merge_names<I>(&self, names: I)
    where
        I: IntoIterator<Item = (u32, String)>,
    {
        let mut cache = self.load_cache();
        let mut changed = false;

        for (app_id, name) in names {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }

            if cache.names.get(&app_id).map(String::as_str) != Some(trimmed) {
                cache.names.insert(app_id, trimmed.to_string());
                changed = true;
            }
        }

        if changed {
            let _ = self.save_cache(&cache);
        }
    }

    fn load_cache(&self) -> NameCacheFile {
        let Ok(content) = fs::read_to_string(&self.cache_path) else {
            return NameCacheFile::default();
        };

        serde_json::from_str(&content).unwrap_or_default()
    }

    fn save_cache(&self, cache: &NameCacheFile) -> Result<(), String> {
        if let Some(parent) = self.cache_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create Steam app name cache directory: {}", e))?;
        }

        let temp_path = self.cache_path.with_extension("tmp");
        let content = serde_json::to_string_pretty(cache)
            .map_err(|e| format!("Failed to serialize Steam app name cache: {}", e))?;

        fs::write(&temp_path, content)
            .map_err(|e| format!("Failed to write Steam app name cache: {}", e))?;
        fs::rename(&temp_path, &self.cache_path)
            .map_err(|e| format!("Failed to apply Steam app name cache: {}", e))
    }

    async fn fetch_missing_names(&self, app_ids: Vec<u32>) -> HashMap<u32, String> {
        let semaphore = Arc::new(Semaphore::new(NAME_RESOLVE_CONCURRENCY));
        let mut tasks = JoinSet::new();

        for app_id in app_ids {
            let client = self.client.clone();
            let semaphore = Arc::clone(&semaphore);

            tasks.spawn(async move {
                let _permit = semaphore.acquire_owned().await.ok();
                let name = Self::fetch_name(client, app_id).await.ok();
                (app_id, name)
            });
        }

        let mut names = HashMap::new();
        while let Some(join_result) = tasks.join_next().await {
            if let Ok((app_id, Some(name))) = join_result {
                names.insert(app_id, name);
            }
        }

        names
    }

    async fn fetch_name(client: reqwest::Client, app_id: u32) -> Result<String, String> {
        let response = client
            .get(STEAM_APPDETAILS_URL)
            .query(&[
                ("appids", app_id.to_string()),
                ("l", "italian".to_string()),
                ("cc", "IT".to_string()),
            ])
            .send()
            .await
            .map_err(|e| format!("Steam app name request failed for {}: {}", app_id, e))?;

        if !response.status().is_success() {
            return Err(format!(
                "Steam app name request returned HTTP error for {}: {}",
                app_id,
                response.status()
            ));
        }

        let data = response
            .json::<HashMap<String, AppDetailsEnvelope>>()
            .await
            .map_err(|e| format!("Failed to parse Steam appdetails for {}: {}", app_id, e))?;

        data
            .get(&app_id.to_string())
            .filter(|envelope| envelope.success)
            .and_then(|envelope| envelope.data.as_ref())
            .and_then(|details| details.get("name"))
            .and_then(|name| name.as_str())
            .map(|name| name.to_string())
            .ok_or_else(|| format!("Steam appdetails did not include a name for {}", app_id))
    }
}
