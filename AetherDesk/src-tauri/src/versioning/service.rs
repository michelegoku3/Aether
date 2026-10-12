use std::collections::HashMap;
use std::sync::Arc;

use tokio::task::JoinSet;

use crate::core::paths::LocalAppPaths;
use crate::manifest::package::ManifestPackageFile;
use crate::manifest::pins::DepotManifestPin;
use crate::versioning::apply::{apply_build_version, ProgressFn};
use crate::versioning::cache::VersionCache;
use crate::versioning::error::VersionError;
use crate::versioning::model::{ApplyVersionReport, BuildInfo, SavedBuild};
use crate::versioning::saved::SavedBuildsStore;
use crate::versioning::snapshot::{older_build_ids, SnapshotAssembler};
use crate::versioning::sources::{
    depotbox::DepotboxSource, steamdb::SteamDbPatchnotesSource, BuildDetailsSource,
    BuildHistorySource,
};

const LOOKUP_BATCH_SIZE: usize = 8;

/// Orchestrates the whole version domain. Depends on source traits (not on
/// implementations) so sources stay swappable and mockable in tests.
pub struct VersionService {
    history_source: Arc<dyn BuildHistorySource>,
    details_source: Arc<dyn BuildDetailsSource>,
    cache: VersionCache,
    saved: SavedBuildsStore,
}

impl VersionService {
    /// Builds the service with the real sources. `token` is the resolved
    /// Depotbox access token (see `resolve_build_details_token`).
    pub fn with_token(token: &str) -> Self {
        Self {
            history_source: Arc::new(SteamDbPatchnotesSource::new()),
            details_source: Arc::new(DepotboxSource::new(token)),
            cache: VersionCache::new(&LocalAppPaths::data_root().join("cache")),
            saved: SavedBuildsStore::new(&LocalAppPaths::data_root()),
        }
    }

    /// Newest-first build list for a game (24 h TTL cache over the feed).
    pub async fn list_builds(&self, app_id: u32) -> Result<Vec<BuildInfo>, VersionError> {
        if let Some(cached) = self.cache.get_build_history(app_id) {
            if !cached.is_empty() {
                return Ok(cached);
            }
        }
        let builds = self.history_source.build_history(app_id).await?;
        if builds.is_empty() {
            return Err(VersionError::BuildHistoryEmpty);
        }
        let _ = self.cache.put_build_history(app_id, builds.clone());
        Ok(builds)
    }

    /// Resolves a BuildID to its pins, serving from cache when fresh.
    async fn resolve_pins(&self, build_id: u64) -> Result<Vec<DepotManifestPin>, VersionError> {
        if let Some(cached) = self.cache.get_build_pins(build_id) {
            crate::desk_log_debug!(
                "versioning",
                "Build {} pins served from cache ({} pin(s))",
                build_id,
                cached.len()
            );
            return Ok(cached);
        }
        crate::desk_log_debug!(
            "versioning",
            "Build {} pins not cached — querying build details source",
            build_id
        );
        let pins = self.details_source.pins_for_build(build_id).await?;
        crate::desk_log_debug!(
            "versioning",
            "Build {} resolved to {} pin(s)",
            build_id,
            pins.len()
        );
        let _ = self.cache.put_build_pins(build_id, pins.clone());
        Ok(pins)
    }

    /// Reconstructs as much of the target snapshot as the available build
    /// history can prove for depots represented in the game's Lua.
    ///
    /// Build details are patch diffs, not full snapshots. The assembler takes
    /// the target diff and then older diffs, keeping the nearest manifest at or
    /// before the target. Depots beyond SteamDB's finite RSS window are omitted
    /// from the result so the Lua writer safely leaves their existing rows
    /// enabled and unchanged; missing historical data is never treated as
    /// evidence that a depot was removed.
    pub async fn resolve_snapshot_pins(
        &self,
        app_id: u32,
        build_id: u64,
        depot_ids: &[u32],
    ) -> Result<Vec<DepotManifestPin>, VersionError> {
        let mut snapshot = SnapshotAssembler::new(depot_ids).ok_or(VersionError::InvalidInput(
            "The game Lua does not contain any manifest depots",
        ))?;

        // An unavailable target is not an empty diff: older pins cannot prove it.
        let target_pins = self.resolve_pins(build_id).await?;
        snapshot.push_diff(&target_pins);

        if !snapshot.is_complete() {
            let history = self.list_builds(app_id).await?;
            let candidates = older_build_ids(&history, build_id);
            for batch in candidates.chunks(LOOKUP_BATCH_SIZE) {
                if snapshot.is_complete() {
                    break;
                }
                let mut results = self.resolve_pins_batch(batch).await.map_err(|error| {
                    VersionError::source(
                        "build snapshot",
                        format!("Cannot reconstruct build {build_id}; historical batch {batch:?} failed: {error}"),
                    )
                })?;
                // Fetch completion order cannot change nearest-build provenance.
                for &candidate in batch {
                    if snapshot.is_complete() {
                        break;
                    }
                    let pins = results.remove(&candidate).ok_or_else(|| {
                        VersionError::source(
                            "build snapshot",
                            format!("Missing historical diff {candidate} for build {build_id}"),
                        )
                    })?.map_err(|error| {
                        VersionError::source(
                            "build snapshot",
                            format!("Cannot reconstruct build {build_id}; required historical diff {candidate} failed: {error}"),
                        )
                    })?;
                    snapshot.push_diff(&pins);
                }
            }
        }

        let unresolved = snapshot.missing_depots();
        let pins = snapshot.into_pins();
        if unresolved.is_empty() {
            crate::desk_log_info!(
                "versioning",
                "Build {} reconstructed as a complete {}-depot snapshot for app {}",
                build_id,
                pins.len(),
                app_id
            );
        } else {
            // The SteamDB RSS feed is intentionally short. Not finding a depot
            // in that window does not prove that it did not exist at the target
            // build. Apply every manifest we could resolve and preserve the
            // existing Lua rows for the rest, matching the safe policy from
            // commit 0a778fb (absence from available diffs is not removal).
            crate::desk_log_warn!(
                "versioning",
                "Build {} for app {} resolved {} depot(s); {} depot(s) were outside the available history and will remain unchanged: {:?}",
                build_id,
                app_id,
                pins.len(),
                unresolved.len(),
                unresolved
            );
        }
        Ok(pins)
    }

    /// Resolves one bounded group concurrently. Cache access is batched too,
    /// avoiding one full JSON read/write cycle per historical build.
    async fn resolve_pins_batch(
        &self,
        build_ids: &[u64],
    ) -> Result<HashMap<u64, Result<Vec<DepotManifestPin>, VersionError>>, VersionError> {
        let cached = self.cache.get_build_pins_many(build_ids);
        let mut results: HashMap<u64, Result<Vec<DepotManifestPin>, VersionError>> = cached
            .iter()
            .map(|(build_id, pins)| (*build_id, Ok(pins.clone())))
            .collect();
        let mut requests = JoinSet::new();

        for &build_id in build_ids {
            if cached.contains_key(&build_id) {
                continue;
            }
            let source = Arc::clone(&self.details_source);
            requests.spawn(async move { (build_id, source.pins_for_build(build_id).await) });
        }

        let mut cache_entries = Vec::new();
        while let Some(joined) = requests.join_next().await {
            let (build_id, result) = joined.map_err(|err| {
                VersionError::source(
                    "build details source",
                    format!("lookup task failed: {err}"),
                )
            })?;
            if let Ok(pins) = &result {
                cache_entries.push((build_id, pins.clone()));
            }
            results.insert(build_id, result);
        }
        if !cache_entries.is_empty() {
            let _ = self.cache.put_build_pins_many(cache_entries);
        }
        Ok(results)
    }

    /// Applies the resolved portion of a build (blocking file I/O — call from
    /// a blocking task). Unresolved depots are intentionally absent from
    /// `pins`, causing the Lua writer to preserve them unchanged.
    pub fn apply_build_sync(
        &self,
        app_id: u32,
        build_id: u64,
        steam_path: &str,
        library_path: &str,
        pins: &[DepotManifestPin],
        generated_manifests: &[ManifestPackageFile],
        progress: &ProgressFn<'_>,
    ) -> Result<ApplyVersionReport, VersionError> {
        apply_build_version(
            app_id,
            build_id,
            steam_path,
            library_path,
            pins,
            generated_manifests,
            progress,
        )
    }

    pub fn list_saved(&self, app_id: u32) -> Vec<SavedBuild> {
        self.saved.list(app_id)
    }

    pub fn save_build(
        &self,
        app_id: u32,
        build_id: u64,
        date: String,
        title: String,
    ) -> Result<SavedBuild, VersionError> {
        self.saved
            .add(SavedBuild {
                app_id,
                build_id,
                date,
                title,
                saved_at: 0, // filled by the store
            })
            .map_err(|e| VersionError::Io {
                context: "saved builds",
                detail: e,
            })
    }

    pub fn remove_saved(&self, app_id: u32, build_id: u64) -> Result<(), VersionError> {
        self.saved
            .remove(app_id, build_id)
            .map_err(|e| VersionError::Io {
                context: "saved builds",
                detail: e,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versioning::sources::BoxFuture;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::Mutex;

    const TARGET: u64 = 2_000_100;
    const NEAR: u64 = TARGET - 1;
    const FAR: u64 = TARGET - 2;

    struct FixtureSource {
        history: Result<Vec<BuildInfo>, VersionError>,
        details: HashMap<u64, Result<Vec<DepotManifestPin>, VersionError>>,
        calls: Mutex<Vec<u64>>,
        history_calls: AtomicUsize,
        panic_at: AtomicU64,
    }

    impl BuildHistorySource for FixtureSource {
        fn build_history(
            &self,
            _app_id: u32,
        ) -> BoxFuture<'_, Result<Vec<BuildInfo>, VersionError>> {
            Box::pin(async move {
                self.history_calls.fetch_add(1, Ordering::Relaxed);
                self.history.clone()
            })
        }
    }

    impl BuildDetailsSource for FixtureSource {
        fn pins_for_build(
            &self,
            build_id: u64,
        ) -> BoxFuture<'_, Result<Vec<DepotManifestPin>, VersionError>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push(build_id);
                assert_ne!(
                    self.panic_at.load(Ordering::Relaxed),
                    build_id,
                    "fixture task failure"
                );
                // Ensure an older response can finish before the nearer one.
                if build_id == NEAR {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                self.details
                    .get(&build_id)
                    .cloned()
                    .unwrap_or(Err(VersionError::BuildNotFound))
            })
        }
    }

    fn pin(depot_id: u32, manifest_id: &str) -> DepotManifestPin {
        DepotManifestPin {
            depot_id,
            manifest_id: manifest_id.to_string(),
        }
    }

    fn build(build_id: u64) -> BuildInfo {
        BuildInfo {
            build_id,
            date: "2026-01-01".into(),
            title: "fixture".into(),
        }
    }

    fn fixture(
        history: Result<Vec<BuildInfo>, VersionError>,
        details: HashMap<u64, Result<Vec<DepotManifestPin>, VersionError>>,
    ) -> (tempfile::TempDir, VersionService, Arc<FixtureSource>) {
        let temp = tempfile::tempdir().unwrap();
        let source = Arc::new(FixtureSource {
            history,
            details,
            calls: Mutex::new(Vec::new()),
            history_calls: AtomicUsize::new(0),
            panic_at: AtomicU64::new(0),
        });
        let service = VersionService {
            history_source: source.clone(),
            details_source: source.clone(),
            cache: VersionCache::new(&temp.path().join("cache")),
            saved: SavedBuildsStore::new(temp.path()),
        };
        (temp, service, source)
    }

    #[tokio::test]
    async fn failed_target_cannot_authorize_older_pins_even_when_they_are_cached() {
        for failure in [
            VersionError::source("fixture", "timeout"),
            VersionError::parse("fixture", "invalid payload"),
            VersionError::BuildNotFound,
        ] {
            let (_temp, service, source) = fixture(
                Ok(vec![build(TARGET), build(NEAR)]),
                HashMap::from([(TARGET, Err(failure)), (NEAR, Ok(vec![pin(101, "10001")]))]),
            );
            service
                .cache
                .put_build_pins(NEAR, vec![pin(101, "10001")])
                .unwrap();
            assert!(service
                .resolve_snapshot_pins(42, TARGET, &[101])
                .await
                .is_err());
            assert_eq!(*source.calls.lock().unwrap(), vec![TARGET]);
            assert_eq!(source.history_calls.load(Ordering::Relaxed), 0);
        }
    }

    #[tokio::test]
    async fn failed_required_intermediate_diff_cannot_be_skipped_for_an_older_pin() {
        let (_temp, service, _source) = fixture(
            Ok(vec![build(TARGET), build(NEAR), build(FAR)]),
            HashMap::from([
                (TARGET, Ok(vec![pin(101, "10001")])),
                (NEAR, Err(VersionError::source("fixture", "HTTP 503"))),
                (FAR, Ok(vec![pin(102, "10200")])),
            ]),
        );
        let error = service
            .resolve_snapshot_pins(42, TARGET, &[101, 102])
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(&NEAR.to_string()),
            "failed interval must identify its provenance"
        );
    }

    #[tokio::test]
    async fn successful_empty_target_diff_can_use_proven_older_changes() {
        let (_temp, service, _source) = fixture(
            Ok(vec![build(TARGET), build(NEAR)]),
            HashMap::from([
                (TARGET, Ok(Vec::new())),
                (NEAR, Ok(vec![pin(101, "10001")])),
            ]),
        );
        assert_eq!(
            service
                .resolve_snapshot_pins(42, TARGET, &[101])
                .await
                .unwrap(),
            vec![pin(101, "10001")]
        );
    }

    #[tokio::test]
    async fn nearer_pins_win_despite_out_of_order_completion_and_unneeded_older_failure() {
        let (_temp, service, _source) = fixture(
            Ok(vec![build(TARGET), build(NEAR), build(FAR)]),
            HashMap::from([
                (TARGET, Ok(vec![pin(101, "10001")])),
                (NEAR, Ok(vec![pin(102, "10201")])),
                (
                    FAR,
                    Err(VersionError::source("fixture", "unneeded old diff failed")),
                ),
            ]),
        );
        assert_eq!(
            service
                .resolve_snapshot_pins(42, TARGET, &[101, 102])
                .await
                .unwrap(),
            vec![pin(101, "10001"), pin(102, "10201")]
        );
    }

    #[tokio::test]
    async fn finite_successful_history_still_returns_only_the_proven_depots() {
        let (_temp, service, _source) = fixture(
            Ok(vec![build(TARGET)]),
            HashMap::from([(TARGET, Ok(vec![pin(101, "10001")]))]),
        );
        assert_eq!(
            service
                .resolve_snapshot_pins(42, TARGET, &[101, 102])
                .await
                .unwrap(),
            vec![pin(101, "10001")]
        );
    }

    #[tokio::test]
    async fn unavailable_history_is_not_known_finite_history() {
        let (_temp, service, _source) = fixture(
            Err(VersionError::source("fixture", "RSS unavailable")),
            HashMap::from([(TARGET, Ok(vec![pin(101, "10001")]))]),
        );
        assert!(service
            .resolve_snapshot_pins(42, TARGET, &[101, 102])
            .await
            .is_err());
    }

    #[tokio::test]
    async fn task_failure_in_required_history_batch_is_propagated() {
        let (_temp, service, source) = fixture(
            Ok(vec![build(TARGET), build(NEAR)]),
            HashMap::from([(TARGET, Ok(Vec::new()))]),
        );
        source.panic_at.store(NEAR, Ordering::Relaxed);
        assert!(service
            .resolve_snapshot_pins(42, TARGET, &[101])
            .await
            .is_err());
    }

    #[tokio::test]
    async fn valid_immutable_target_cache_needs_no_remote_fallback() {
        let (_temp, service, source) = fixture(
            Err(VersionError::BuildHistoryEmpty),
            HashMap::from([(TARGET, Err(VersionError::TokenInvalid))]),
        );
        service
            .cache
            .put_build_pins(TARGET, vec![pin(101, "10001")])
            .unwrap();
        assert_eq!(
            service
                .resolve_snapshot_pins(42, TARGET, &[101])
                .await
                .unwrap(),
            vec![pin(101, "10001")]
        );
        assert!(source.calls.lock().unwrap().is_empty());
        assert_eq!(source.history_calls.load(Ordering::Relaxed), 0);
    }
}
