use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use crate::providers::http;
use crate::steam::api::{self, AppDetailsEnvelope};

const NAME_RESOLVE_TIMEOUT_SECONDS: u64 = 4;
const NAME_RESOLVE_CONCURRENCY: usize = 8;
const CACHE_FILE_NAME: &str = "steam_app_names.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct NameCacheFile {
    #[serde(default)]
    names: HashMap<u32, String>,
    #[serde(default)]
    image_urls: HashMap<u32, String>,
    #[serde(default)]
    hero_image_urls: HashMap<u32, String>,
}

#[derive(Debug, Clone)]
struct AppNameDetails {
    name: String,
    image_url: Option<String>,
    hero_image_url: Option<String>,
}

/// Resolves Steam app names and lightweight image URLs by App ID and stores
/// them in a persistent cache.
///
/// The Library view is based on local Lua files, whose comments can sometimes
/// contain noisy/wrong titles and whose App IDs often need hashed Steam image
/// URLs that cannot be derived from `appid` alone. This resolver mirrors the
/// Store source of truth by asking Steam appdetails, then caching the canonical
/// name + best available capsule/header URL so later Library opens are fast.
pub struct SteamAppNameResolver {
    cache_path: PathBuf,
    client: reqwest::Client,
}

impl SteamAppNameResolver {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_path: cache_dir.join(CACHE_FILE_NAME),
            client: http::build_client(NAME_RESOLVE_TIMEOUT_SECONDS),
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

    /// Returns cached cover/capsule URLs without doing network I/O.
    pub fn cached_image_urls(&self, app_ids: Vec<u32>) -> HashMap<u32, String> {
        let cache = self.load_cache();
        Self::filter_cached_images(cache, app_ids)
    }

    /// Returns cached landscape/header URLs without doing network I/O.
    pub fn cached_hero_image_urls(&self, app_ids: Vec<u32>) -> HashMap<u32, String> {
        let cache = self.load_cache();
        Self::filter_cached_heroes(cache, app_ids)
    }

    /// Resolves missing names/images through Steam and persists them for future
    /// cache-only reads. Use this from warm-up/background paths, not from
    /// UI-critical commands.
    pub async fn resolve_names(&self, app_ids: Vec<u32>) -> HashMap<u32, String> {
        let mut cache = self.load_cache();
        let unique_ids = Self::unique_app_ids(app_ids);

        let missing_ids: Vec<u32> = unique_ids
            .iter()
            .copied()
            .filter(|app_id| {
                !cache.names.contains_key(app_id)
                    || !cache.image_urls.contains_key(app_id)
                    || !cache.hero_image_urls.contains_key(app_id)
            })
            .collect();

        if !missing_ids.is_empty() {
            let fetched = self.fetch_missing_details(missing_ids).await;
            let mut changed = false;

            for (app_id, details) in fetched {
                let trimmed = details.name.trim();
                if !trimmed.is_empty() && cache.names.get(&app_id).map(String::as_str) != Some(trimmed) {
                    cache.names.insert(app_id, trimmed.to_string());
                    changed = true;
                }
                if let Some(image_url) = details.image_url.as_deref().map(str::trim).filter(|url| !url.is_empty()) {
                    if cache.image_urls.get(&app_id).map(String::as_str) != Some(image_url) {
                        cache.image_urls.insert(app_id, image_url.to_string());
                        changed = true;
                    }
                }
                if let Some(hero_image_url) = details.hero_image_url.as_deref().map(str::trim).filter(|url| !url.is_empty()) {
                    if cache.hero_image_urls.get(&app_id).map(String::as_str) != Some(hero_image_url) {
                        cache.hero_image_urls.insert(app_id, hero_image_url.to_string());
                        changed = true;
                    }
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

    fn filter_cached_images(cache: NameCacheFile, app_ids: Vec<u32>) -> HashMap<u32, String> {
        Self::unique_app_ids(app_ids)
            .into_iter()
            .filter_map(|app_id| {
                cache
                    .image_urls
                    .get(&app_id)
                    .cloned()
                    .filter(|url| !url.trim().is_empty())
                    .map(|url| (app_id, url))
            })
            .collect()
    }

    fn filter_cached_heroes(cache: NameCacheFile, app_ids: Vec<u32>) -> HashMap<u32, String> {
        Self::unique_app_ids(app_ids)
            .into_iter()
            .filter_map(|app_id| {
                cache
                    .hero_image_urls
                    .get(&app_id)
                    .cloned()
                    .filter(|url| !url.trim().is_empty())
                    .map(|url| (app_id, url))
            })
            .collect()
    }

    /// Merges trusted image URLs obtained from another local/live source into
    /// the shared app-name cache used by the Library.
    pub fn merge_image_urls<I>(&self, image_urls: I)
    where
        I: IntoIterator<Item = (u32, String)>,
    {
        let mut cache = self.load_cache();
        let mut changed = false;

        for (app_id, image_url) in image_urls {
            let trimmed = image_url.trim();
            if trimmed.is_empty() {
                continue;
            }
            if cache.image_urls.get(&app_id).map(String::as_str) != Some(trimmed) {
                cache.image_urls.insert(app_id, trimmed.to_string());
                changed = true;
            }
        }

        if changed {
            let _ = self.save_cache(&cache);
        }
    }

    /// Merges trusted landscape/header URLs obtained from another local/live
    /// source into the shared app-name cache used by the Library.
    pub fn merge_hero_image_urls<I>(&self, hero_image_urls: I)
    where
        I: IntoIterator<Item = (u32, String)>,
    {
        let mut cache = self.load_cache();
        let mut changed = false;

        for (app_id, hero_image_url) in hero_image_urls {
            let trimmed = hero_image_url.trim();
            if trimmed.is_empty() {
                continue;
            }
            if cache.hero_image_urls.get(&app_id).map(String::as_str) != Some(trimmed) {
                cache.hero_image_urls.insert(app_id, trimmed.to_string());
                changed = true;
            }
        }

        if changed {
            let _ = self.save_cache(&cache);
        }
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

    async fn fetch_missing_details(&self, app_ids: Vec<u32>) -> HashMap<u32, AppNameDetails> {
        let client = self.client.clone();
        api::concurrent_app_tasks(
            app_ids,
            NAME_RESOLVE_CONCURRENCY,
            move |app_id| {
                let client = client.clone();
                async move {
                    let details = Self::fetch_details(&client, app_id).await.ok();
                    details.map(|details| (app_id, details))
                }
            },
        )
        .await
    }

    async fn fetch_details(client: &reqwest::Client, app_id: u32) -> Result<AppNameDetails, String> {
        let envelope: AppDetailsEnvelope = api::fetch_app_details(client, app_id).await?;
        let data = envelope
            .data
            .as_ref()
            .ok_or_else(|| format!("Steam appdetails did not include data for {}", app_id))?;

        let name = data
            .get("name")
            .and_then(|name| name.as_str())
            .map(|name| name.to_string())
            .ok_or_else(|| format!("Steam appdetails did not include a name for {}", app_id))?;

        // Cover URL must be a capsule, never a header. `header_image` is a wide
        // landscape "hero" banner and was leaking into the capsule slot after a
        // cache clean (reported as "hero but in the capsule slot").
        let image_url = ["capsule_image", "capsule_imagev5"]
            .iter()
            .find_map(|key| {
                data.get(*key)
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            });
        let hero_image_url = ["header_image", "background_raw", "background"]
            .iter()
            .find_map(|key| {
                data.get(*key)
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            });

        Ok(AppNameDetails { name, image_url, hero_image_url })
    }
}

/// Helper that synchronously retrieves the cached game title for an AppID without network I/O.
pub fn get_cached_game_name(app_id: u32) -> String {
    let cache_dir = crate::core::paths::LocalAppPaths::data_root().join("cache");
    let resolver = SteamAppNameResolver::new(cache_dir);
    resolver
        .cached_names(vec![app_id])
        .get(&app_id)
        .cloned()
        .unwrap_or_else(|| "Unknown Game".to_string())
}
