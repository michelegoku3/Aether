//! Persistent memory for terminal Workshop failures.
//!
//! A Hubcap `404` for one `(workshop_id, manifest_gid)` is a data-availability
//! answer, not a transient error. Remembering it keeps a removed item from
//! consuming a generation slot on every pass. A changed manifest retries
//! immediately.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::paths::LocalAppPaths;

const WORKSHOP_FAILURE_FILE: &str = "workshop_unavailable.json";
const WORKSHOP_FAILURE_TTL_SECONDS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkshopFailure {
    manifest_gid: String,
    failed_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct WorkshopFailureCache {
    #[serde(default)]
    items: HashMap<u64, WorkshopFailure>,
}

impl WorkshopFailureCache {
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        let Ok(content) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&content).unwrap_or_else(|error| {
            crate::desk_log_warn!(
                "workshop",
                "Ignoring corrupt Workshop failure cache {}: {}",
                path.display(),
                error
            );
            Self::default()
        })
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let content = serde_json::to_vec_pretty(self)
            .map_err(|error| format!("Could not serialize Workshop failure cache: {error}"))?;
        crate::core::state_io::write_atomic(path, &content)
    }

    pub fn is_unavailable(&self, workshop_id: u64, manifest_gid: u64, now: u64) -> bool {
        let Some(entry) = self.items.get(&workshop_id) else {
            return false;
        };
        entry.manifest_gid.parse::<u64>().ok() == Some(manifest_gid)
            && now.saturating_sub(entry.failed_at) < WORKSHOP_FAILURE_TTL_SECONDS
    }

    pub fn record(&mut self, workshop_id: u64, manifest_gid: u64, now: u64) {
        self.items.insert(
            workshop_id,
            WorkshopFailure {
                manifest_gid: manifest_gid.to_string(),
                failed_at: now,
            },
        );
    }

    pub fn prune(&mut self, now: u64) {
        self.items
            .retain(|_, entry| now.saturating_sub(entry.failed_at) < WORKSHOP_FAILURE_TTL_SECONDS);
    }

    fn path() -> PathBuf {
        LocalAppPaths::state_dir().join(WORKSHOP_FAILURE_FILE)
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

/// `None` when the error is not an HTTP failure (network/timeout/local error).
pub fn http_status_from_error(error: &str) -> Option<u16> {
    let marker = "HTTP ";
    let start = error.rfind(marker)? + marker.len();
    let digits: String = error[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    if digits.len() != 3 {
        return None;
    }
    digits.parse().ok()
}

/// A Hubcap `404` means the item/manifest is not in the database.
pub fn is_hubcap_not_found(error: &str) -> bool {
    http_status_from_error(error) == Some(404)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_792_492_000;
    const ITEM: u64 = 3_806_068_832;
    const MANIFEST: u64 = 3_748_992_252_827_399_921;

    #[test]
    fn cache_expires_after_24_hours() {
        let mut cache = WorkshopFailureCache::default();
        cache.record(ITEM, MANIFEST, NOW);

        assert!(cache.is_unavailable(ITEM, MANIFEST, NOW));
        assert!(cache.is_unavailable(ITEM, MANIFEST, NOW + WORKSHOP_FAILURE_TTL_SECONDS - 1));
        assert!(!cache.is_unavailable(ITEM, MANIFEST, NOW + WORKSHOP_FAILURE_TTL_SECONDS));
    }

    #[test]
    fn changed_manifest_retries_immediately() {
        let mut cache = WorkshopFailureCache::default();
        cache.record(ITEM, MANIFEST, NOW);

        assert!(!cache.is_unavailable(ITEM, MANIFEST + 1, NOW));
    }

    #[test]
    fn corrupt_state_loads_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("workshop_unavailable.json");
        fs::write(&path, b"{not json").unwrap();

        let cache = WorkshopFailureCache::load_from(&path);

        assert!(!cache.is_unavailable(ITEM, MANIFEST, NOW));
        cache.save_to(&path).unwrap();
        assert!(WorkshopFailureCache::load_from(&path).items.is_empty());
    }

    #[test]
    fn round_trips_state() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("workshop_unavailable.json");
        let mut cache = WorkshopFailureCache::default();
        cache.record(ITEM, MANIFEST, NOW);
        cache.save_to(&path).unwrap();

        let loaded = WorkshopFailureCache::load_from(&path);
        assert!(loaded.is_unavailable(ITEM, MANIFEST, NOW + 10));
    }

    #[test]
    fn classifies_http_404_only() {
        assert_eq!(
            http_status_from_error("Hubcap Workshop manifest request http_error (HTTP 404)"),
            Some(404)
        );
        assert!(is_hubcap_not_found(
            "Hubcap Workshop manifest request http_error (HTTP 404)"
        ));
        assert!(!is_hubcap_not_found(
            "Hubcap Workshop manifest request http_error (HTTP 403)"
        ));
        assert!(!is_hubcap_not_found(
            "Hubcap Workshop manifest request provider_unavailable (HTTP 503)"
        ));
        assert!(!is_hubcap_not_found("network timeout"));
        assert_eq!(http_status_from_error("HTTP 4040"), None);
    }

    #[test]
    fn prune_drops_expired_entries() {
        let mut cache = WorkshopFailureCache::default();
        cache.record(ITEM, MANIFEST, NOW);
        cache.record(ITEM + 1, MANIFEST, NOW + WORKSHOP_FAILURE_TTL_SECONDS);

        cache.prune(NOW + WORKSHOP_FAILURE_TTL_SECONDS);

        assert!(!cache.is_unavailable(ITEM, MANIFEST, NOW + WORKSHOP_FAILURE_TTL_SECONDS));
        assert!(cache.is_unavailable(ITEM + 1, MANIFEST, NOW + WORKSHOP_FAILURE_TTL_SECONDS));
    }
}
