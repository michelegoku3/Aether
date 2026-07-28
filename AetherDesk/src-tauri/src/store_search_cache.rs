use crate::store_service::UnifiedStoreGame;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const CACHE_FILE_NAME: &str = "store_search_cache.json";
const CACHE_SCHEMA_VERSION: u32 = 1;
const FRESH_TTL_SECONDS: u64 = 24 * 60 * 60;

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreSearchCacheFile {
    version: u32,
    entries: HashMap<String, StoreSearchCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoreSearchCacheEntry {
    updated_at_unix: u64,
    results: Vec<UnifiedStoreGame>,
}

/// Persistent query cache for Store searches.
///
/// It is deliberately isolated from StoreService: StoreService owns live search/merge/ranking,
/// while this repository owns only normalized query keys and JSON persistence.
pub struct StoreSearchCache {
    cache_path: PathBuf,
}

impl StoreSearchCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_path: cache_dir.join(CACHE_FILE_NAME),
        }
    }

    pub fn get_fresh(&self, query: &str) -> Option<Vec<UnifiedStoreGame>> {
        let key = Self::normalize_query(query)?;
        let cache = self.load_cache();
        let entry = cache.entries.get(&key)?;

        if Self::now_unix().saturating_sub(entry.updated_at_unix) <= FRESH_TTL_SECONDS {
            Some(entry.results.clone())
        } else {
            None
        }
    }

    pub fn get_any(&self, query: &str) -> Option<Vec<UnifiedStoreGame>> {
        let key = Self::normalize_query(query)?;
        self.load_cache()
            .entries
            .get(&key)
            .map(|entry| entry.results.clone())
    }

    pub fn put(&self, query: &str, results: Vec<UnifiedStoreGame>) -> Result<(), String> {
        let Some(key) = Self::normalize_query(query) else {
            return Ok(());
        };

        let mut cache = self.load_cache();
        cache.version = CACHE_SCHEMA_VERSION;
        cache.entries.insert(
            key,
            StoreSearchCacheEntry {
                updated_at_unix: Self::now_unix(),
                results,
            },
        );

        self.save_cache(&cache)
    }

    fn normalize_query(query: &str) -> Option<String> {
        let normalized = query
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_lowercase();

        (!normalized.is_empty()).then_some(normalized)
    }

    fn load_cache(&self) -> StoreSearchCacheFile {
        let Ok(content) = fs::read_to_string(&self.cache_path) else {
            return StoreSearchCacheFile {
                version: CACHE_SCHEMA_VERSION,
                entries: HashMap::new(),
            };
        };

        let mut cache = serde_json::from_str::<StoreSearchCacheFile>(&content).unwrap_or_default();
        if cache.version != CACHE_SCHEMA_VERSION {
            cache.entries.clear();
            cache.version = CACHE_SCHEMA_VERSION;
        }
        cache
    }

    fn save_cache(&self, cache: &StoreSearchCacheFile) -> Result<(), String> {
        if let Some(parent) = self.cache_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create Store search cache directory: {}", e))?;
        }

        let temp_path = self.cache_path.with_extension("tmp");
        let content = serde_json::to_string_pretty(cache)
            .map_err(|e| format!("Failed to serialize Store search cache: {}", e))?;

        fs::write(&temp_path, content)
            .map_err(|e| format!("Failed to write Store search cache: {}", e))?;
        fs::rename(&temp_path, &self.cache_path)
            .map_err(|e| format!("Failed to apply Store search cache: {}", e))
    }

    fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default()
    }
}
