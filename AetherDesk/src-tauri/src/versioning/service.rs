use std::sync::Arc;

use crate::core::paths::LocalAppPaths;
use crate::manifest::pins::DepotManifestPin;
use crate::versioning::apply::{apply_build_version, ProgressFn};
use crate::versioning::cache::VersionCache;
use crate::versioning::error::VersionError;
use crate::versioning::model::{ApplyVersionReport, BuildInfo, SavedBuild};
use crate::versioning::saved::SavedBuildsStore;
use crate::versioning::sources::{
    self, depotbox::DepotboxSource, steamdb::SteamDbPatchnotesSource, BuildDetailsSource,
    BuildHistorySource,
};

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
        let token = token
            .unwrap_or_else(|| sources::DEFAULT_BUILD_DETAILS_TOKEN.to_string());
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
    pub async fn resolve_pins(&self, build_id: u64) -> Result<Vec<DepotManifestPin>, VersionError> {
        if let Some(cached) = self.cache.get_build_pins(build_id) {
            crate::desk_log_debug!("versioning", "Build {} pins served from cache ({} pin(s))", build_id, cached.len());
            return Ok(cached);
        }
        crate::desk_log_debug!("versioning", "Build {} pins not cached — querying Depotbox", build_id);
        let pins = self.details_source.pins_for_build(build_id).await?;
        crate::desk_log_debug!("versioning", "Build {} resolved to {} pin(s)", build_id, pins.len());
        let _ = self.cache.put_build_pins(build_id, pins.clone());
        Ok(pins)
    }

    /// Applies a build (blocking file I/O — call from a blocking task).
    /// `pins` must come from `resolve_pins` first, so the network work stays
    /// async while the file pipeline runs off the async runtime.
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
