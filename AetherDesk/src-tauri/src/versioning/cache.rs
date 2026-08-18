use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::manifest::pins::DepotManifestPin;
use crate::versioning::model::BuildInfo;

const CACHE_FILE_NAME: &str = "versioning_cache.json";
const CACHE_SCHEMA_VERSION: u32 = 1;

/// Build history is refreshed daily (the feed only changes on new builds).
pub const BUILD_HISTORY_TTL_SECS: u64 = 24 * 60 * 60;
/// A BuildID → pins mapping is immutable: long TTL, retried only on failure.
pub const BUILD_PINS_TTL_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TimedEntry<T> {
    fetched_at: u64,
    value: T,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    build_history: HashMap<String, TimedEntry<Vec<BuildInfo>>>,
    #[serde(default)]
    build_pins: HashMap<String, TimedEntry<Vec<DepotManifestPin>>>,
}

/// On-disk TTL cache for remote version data. Same shape/strategy as
/// `game_info::cache::GameInfoCache` (schema version + app version + TTLs),
/// so behaviour stays consistent across domains.
pub struct VersionCache {
    path: PathBuf,
}

impl VersionCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            path: cache_dir.join(CACHE_FILE_NAME),
        }
    }

    pub fn get_build_history(&self, app_id: u32) -> Option<Vec<BuildInfo>> {
        let cache = self.load()?;
        let entry = cache.build_history.get(&app_id.to_string())?;
        if now_unix().saturating_sub(entry.fetched_at) > BUILD_HISTORY_TTL_SECS {
            return None;
        }
        Some(entry.value.clone())
    }

    pub fn put_build_history(&self, app_id: u32, builds: Vec<BuildInfo>) -> Result<(), String> {
        let mut cache = self.load().unwrap_or_default();
        cache.schema_version = CACHE_SCHEMA_VERSION;
        cache.build_history.insert(
            app_id.to_string(),
            TimedEntry {
                fetched_at: now_unix(),
                value: builds,
            },
        );
        self.save(&cache)
    }

    pub fn get_build_pins(&self, build_id: u64) -> Option<Vec<DepotManifestPin>> {
        let cache = self.load()?;
        let entry = cache.build_pins.get(&build_id.to_string())?;
        if now_unix().saturating_sub(entry.fetched_at) > BUILD_PINS_TTL_SECS {
            return None;
        }
        Some(entry.value.clone())
    }

    pub fn put_build_pins(&self, build_id: u64, pins: Vec<DepotManifestPin>) -> Result<(), String> {
        let mut cache = self.load().unwrap_or_default();
        cache.schema_version = CACHE_SCHEMA_VERSION;
        cache.build_pins.insert(
            build_id.to_string(),
            TimedEntry {
                fetched_at: now_unix(),
                value: pins,
            },
        );
        self.save(&cache)
    }

    fn load(&self) -> Option<CacheFile> {
        let content = fs::read_to_string(&self.path).ok()?;
        match serde_json::from_str::<CacheFile>(&content) {
            Ok(cache) if cache.schema_version == CACHE_SCHEMA_VERSION => Some(cache),
            _ => None,
        }
    }

    fn save(&self, cache: &CacheFile) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create cache dir: {e}"))?;
        }
        let json = serde_json::to_string_pretty(cache)
            .map_err(|e| format!("Failed to serialize version cache: {e}"))?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json)
            .map_err(|e| format!("Failed to write version cache: {e}"))?;
        fs::rename(&tmp, &self.path)
            .map_err(|e| format!("Failed to commit version cache: {e}"))
    }
}

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
