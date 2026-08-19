use std::collections::HashMap;
use std::sync::Arc;

use tokio::task::JoinSet;

use crate::core::paths::LocalAppPaths;
use crate::manifest::pins::DepotManifestPin;
use crate::versioning::apply::{apply_build_version, ProgressFn};
use crate::versioning::cache::VersionCache;
use crate::versioning::error::VersionError;
use crate::versioning::model::{ApplyVersionReport, BuildInfo, SavedBuild};
use crate::versioning::saved::SavedBuildsStore;
use crate::versioning::snapshot::{older_build_ids, SnapshotAssembler};
use crate::versioning::sources::{
    self, depotbox::DepotboxSource, steamdb::SteamDbPatchnotesSource, BuildDetailsSource,
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
    /// Builds the service with the real sources. `token` is the Depotbox
    /// access token; `None` falls back to the built-in default.
    pub fn with_token(token: Option<String>) -> Self {
        let token = token.unwrap_or_else(|| sources::DEFAULT_BUILD_DETAILS_TOKEN.to_string());
        Self {
            history_source: Arc::new(SteamDbPatchnotesSource::new()),
            details_source: Arc::new(DepotboxSource::new(token)),
            cache: VersionCache::new(LocalAppPaths::data_root().join("cache")),
            saved: SavedBuildsStore::new(LocalAppPaths::data_root()),
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

    /// Reconstructs the complete manifest snapshot visible at `build_id` for
    /// the depots present in the game's Lua.
    ///
    /// Build details are patch diffs, not full snapshots. The assembler takes
    /// the target diff and then older diffs until every required depot has its
    /// nearest manifest at or before the target.
    pub async fn resolve_snapshot_pins(
        &self,
        app_id: u32,
        build_id: u64,
        depot_ids: &[u32],
    ) -> Result<Vec<DepotManifestPin>, VersionError> {
        let mut snapshot = SnapshotAssembler::new(depot_ids).ok_or(
            VersionError::InvalidInput("The game Lua does not contain any manifest depots"),
        )?;

        snapshot.push_diff(&self.resolve_pins(build_id).await?);
        if !snapshot.is_complete() {
            let history = self.list_builds(app_id).await?;
            let candidates = older_build_ids(&history, build_id);

            for batch in candidates.chunks(LOOKUP_BATCH_SIZE) {
                if snapshot.is_complete() {
                    break;
                }
                let mut results = self.resolve_pins_batch(batch).await?;

                // Requests finish in arbitrary order; consume them in the
                // candidate order to preserve nearest-build semantics.
                for &candidate in batch {
                    if snapshot.is_complete() {
                        break;
                    }
                    let result = results.remove(&candidate).ok_or_else(|| {
                        VersionError::source(
                            "build details source",
                            format!("build {candidate} lookup produced no result"),
                        )
                    })?;
                    let pins = result?;
                    snapshot.push_diff(&pins);
                }
            }
        }

        if !snapshot.is_complete() {
            return Err(VersionError::IncompleteSnapshot(
                snapshot.missing_depots(),
            ));
        }

        let pins = snapshot.into_pins();
        crate::desk_log_info!(
            "versioning",
            "Build {} reconstructed as a complete {}-depot snapshot for app {}",
            build_id,
            pins.len(),
            app_id
        );
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

    /// Applies a build (blocking file I/O — call from a blocking task).
    /// `pins` must be a complete snapshot from `resolve_snapshot_pins`, so the
    /// network work stays async while file I/O runs off the async runtime.
    pub fn apply_build_sync(
        &self,
        app_id: u32,
        build_id: u64,
        steam_path: &str,
        library_path: &str,
        pins: &[DepotManifestPin],
        progress: &ProgressFn<'_>,
    ) -> Result<ApplyVersionReport, VersionError> {
        apply_build_version(app_id, build_id, steam_path, library_path, pins, progress)
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
