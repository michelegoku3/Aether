use crate::store::service::UnifiedStoreGame;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CACHE_FILE_NAME: &str = "store_search_cache.json";
const CACHE_SCHEMA_VERSION: &str = "store-cache-v3";
const FRESH_TTL_SECONDS: u64 = 4 * 60 * 60;
const STALE_RETENTION_SECONDS: u64 = 14 * 24 * 60 * 60;
/// Hard cap on cached queries: keeps the JSON file (rewritten on every `put`)
/// bounded no matter how much the user searches.
const MAX_CACHE_ENTRIES: usize = 200;

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreSearchCacheFile {
    /// Written from `tauri.conf.json`'s version at save time
    /// (`AppHandle::package_info().version`). A cache written by a different
    /// AetherDesk build is ignored on load — self-describing invalidation,
    /// no sidecar stamp file needed. `#[serde(default)]` keeps pre-versioned
    /// files parseable as "unknown app" (which then mismatch and reset).
    #[serde(default)]
    app_version: String,
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
    /// Version of the running build (`tauri.conf.json`), read once per command.
    app_version: String,
}

impl StoreSearchCache {
    pub fn new(cache_dir: &Path, app_version: &str) -> Self {
        Self {
            cache_path: cache_dir.join(CACHE_FILE_NAME),
            app_version: format!("{}|{}", app_version, CACHE_SCHEMA_VERSION),
        }
    }

    pub fn get_fresh(&self, query: &str) -> Option<Vec<UnifiedStoreGame>> {
        self.get_fresh_for(query, FRESH_TTL_SECONDS)
    }

    pub fn get_fresh_for(&self, query: &str, ttl_seconds: u64) -> Option<Vec<UnifiedStoreGame>> {
        let key = Self::normalize_query(query)?;
        let cache = self.load_cache();
        let entry = cache.entries.get(&key)?;

        if Self::now_unix().saturating_sub(entry.updated_at_unix) <= ttl_seconds {
            Some(entry.results.clone())
        } else {
            None
        }
    }

    pub fn get_stale(&self, query: &str) -> Option<Vec<UnifiedStoreGame>> {
        let key = Self::normalize_query(query)?;
        let cache = self.load_cache();
        let entry = cache.entries.get(&key)?;

        if Self::now_unix().saturating_sub(entry.updated_at_unix) <= STALE_RETENTION_SECONDS {
            Some(entry.results.clone())
        } else {
            None
        }
    }

    pub fn get_any(&self, query: &str) -> Option<Vec<UnifiedStoreGame>> {
        self.get_stale(query)
    }

    pub fn put(&self, query: &str, results: Vec<UnifiedStoreGame>) -> Result<(), String> {
        let Some(key) = Self::normalize_query(query) else {
            return Ok(());
        };

        let mut cache = self.load_cache();
        cache.app_version = self.app_version.clone();
        let now = Self::now_unix();
        cache.entries.insert(
            key,
            StoreSearchCacheEntry {
                updated_at_unix: now,
                results,
            },
        );
        prune_entries(&mut cache.entries, now);

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
                app_version: self.app_version.clone(),
                entries: HashMap::new(),
            };
        };

        let mut cache = serde_json::from_str::<StoreSearchCacheFile>(&content).unwrap_or_default();
        // Self-describing invalidation: entries are only usable when written by
        // the same AetherDesk build. Any other build resets the file on load
        // (entries re-fill lazily as searches run).
        if cache.app_version != self.app_version {
            cache.entries.clear();
            cache.app_version = self.app_version.clone();
        }
        cache
    }

    fn save_cache(&self, cache: &StoreSearchCacheFile) -> Result<(), String> {
        if let Some(parent) = self.cache_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create Store search cache directory: {}", e))?;
        }

        let temp_path = self.cache_path.with_extension("tmp");
        let mut stamped = StoreSearchCacheFile {
            app_version: self.app_version.clone(),
            entries: cache.entries.clone(),
        };
        stamped.entries.shrink_to_fit();
        let content = serde_json::to_string_pretty(&stamped)
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

/// Drops entries older than the stale retention and, if still over the cap,
/// evicts the least-recently-written ones. Keeps the cache file bounded.
fn prune_entries(entries: &mut HashMap<String, StoreSearchCacheEntry>, now: u64) {
    entries.retain(|_, entry| now.saturating_sub(entry.updated_at_unix) <= STALE_RETENTION_SECONDS);
    if entries.len() > MAX_CACHE_ENTRIES {
        let mut keys: Vec<(String, u64)> = entries
            .iter()
            .map(|(k, e)| (k.clone(), e.updated_at_unix))
            .collect();
        keys.sort_by_key(|(_, ts)| *ts);
        for (oldest, _) in keys.iter().take(entries.len() - MAX_CACHE_ENTRIES) {
            entries.remove(oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ts: u64) -> StoreSearchCacheEntry {
        StoreSearchCacheEntry { updated_at_unix: ts, results: Vec::new() }
    }

    #[test]
    fn prune_drops_stale_and_caps_at_max_entries() {
        let now = 10_000_000;
        let mut entries: HashMap<String, StoreSearchCacheEntry> = HashMap::new();
        // One stale entry (older than retention).
        entries.insert("stale".to_string(), entry(now - STALE_RETENTION_SECONDS - 1));
        // MAX + 10 fresh entries with increasing timestamps.
        for i in 0..(MAX_CACHE_ENTRIES + 10) {
            entries.insert(format!("q{i}"), entry(now - i as u64));
        }
        prune_entries(&mut entries, now);
        assert!(!entries.contains_key("stale"));
        assert_eq!(entries.len(), MAX_CACHE_ENTRIES);
        // The oldest fresh filler ("q209") was evicted, the newest kept.
        assert!(!entries.contains_key(&format!("q{}", MAX_CACHE_ENTRIES + 9)));
        assert!(entries.contains_key("q0"));
    }
}
