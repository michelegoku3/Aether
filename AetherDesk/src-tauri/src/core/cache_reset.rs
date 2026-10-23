//! One owner for "Clear AetherDesk caches".
//!
//! Deleting the cache folder alone leaves every process-lifetime cache alive,
//! so the next search or update check can still answer from stale data. This
//! module drops the in-memory caches first, then removes the on-disk folder.
//! New caches must add their clear call here; the architecture invariant lives
//! in `AetherDesk/docs/ARCHITECTURE.md`.

use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheClearReport {
    /// Files removed from `<AetherData>/cache`.
    pub files: usize,
    /// Entries dropped from process-lifetime caches.
    pub memory: usize,
}

/// Clears every cache outside the source of truth.
///
/// Order is deliberate: in-memory values are dropped before the folder
/// disappears, so no reader can serve a stale answer while the delete runs.
/// A file-system failure still reports that the process caches were cleared.
pub async fn clear_all() -> Result<CacheClearReport, String> {
    let memory = clear_memory_caches().await;
    let cache_dir = crate::core::paths::LocalAppPaths::data_root().join("cache");
    let files = clear_cache_dir(&cache_dir).map_err(|error| {
        format!(
            "In-memory caches were cleared, but the cache folder {} could not be removed: {error}",
            cache_dir.display()
        )
    })?;
    Ok(CacheClearReport { files, memory })
}

async fn clear_memory_caches() -> usize {
    let mut removed = 0;
    removed += crate::steam::store_items::clear_meta_cache();
    removed += crate::steam::store::clear_suggest_cache();
    removed += crate::updater::github::clear_release_cache();
    removed += crate::providers::hubcap::auth::clear_validation_cache().await;
    removed += usize::from(crate::providers::hubcap_generation::invalidate_usage_snapshot().await);
    removed
}

fn clear_cache_dir(cache_dir: &Path) -> Result<usize, String> {
    if !cache_dir.is_dir() {
        std::fs::create_dir_all(cache_dir)
            .map_err(|error| format!("Could not create {}: {error}", cache_dir.display()))?;
        return Ok(0);
    }

    let files = count_entries(cache_dir);
    std::fs::remove_dir_all(cache_dir)
        .map_err(|error| format!("Could not remove {}: {error}", cache_dir.display()))?;
    std::fs::create_dir_all(cache_dir)
        .map_err(|error| format!("Could not recreate {}: {error}", cache_dir.display()))?;
    Ok(files)
}

/// Counts files before the delete; the folder holds JSON caches and, in a
/// portable install, a few small subdirectories.
fn count_entries(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                count_entries(&path)
            } else {
                1
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn clear_cache_dir_counts_files_and_recreates_the_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = dir.path().join("cache");
        fs::create_dir_all(cache.join("nested")).expect("nested");
        fs::write(cache.join("a.json"), b"{}").expect("a");
        fs::write(cache.join("nested/b.json"), b"{}").expect("b");

        assert_eq!(clear_cache_dir(&cache).expect("clear"), 2);
        assert!(cache.is_dir());
        assert_eq!(fs::read_dir(&cache).expect("read").count(), 0);
    }

    #[test]
    fn clear_cache_dir_creates_a_missing_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = dir.path().join("missing");

        assert_eq!(clear_cache_dir(&cache).expect("clear"), 0);
        assert!(cache.is_dir());
    }
}
