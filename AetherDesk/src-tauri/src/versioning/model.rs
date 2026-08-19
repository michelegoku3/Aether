use serde::{Deserialize, Serialize};

/// One entry of a game's build history (from the SteamDB PatchnotesRSS feed).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    pub build_id: u64,
    /// ISO date `YYYY-MM-DD` of the build publication.
    pub date: String,
    /// Human label from the feed (e.g. "Counter-Strike 2 update for 12 August 2026").
    pub title: String,
}

/// Outcome of `apply_game_version` — always the real state that was reached.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyVersionReport {
    /// Number of manifest pins written into the Lua.
    pub applied_pins: usize,
    /// How many of the pinned `.manifest` files were already present locally.
    pub manifests_found: usize,
    /// `"depot:manifest"` pairs still missing from the depotcache folders.
    pub manifests_missing: Vec<String>,
    /// True when the ACF was updated right now.
    pub acf_synced_now: bool,
    /// True when the ACF edit was queued (ACF missing or held by Steam) and
    /// will be retried automatically in the background.
    pub acf_queued: bool,
    /// Restore point of the Lua, if a backup was written.
    pub lua_backup_path: Option<String>,
}

/// A build the user bookmarked for quick access.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedBuild {
    pub app_id: u32,
    pub build_id: u64,
    pub date: String,
    pub title: String,
    /// Unix timestamp of the last save.
    pub saved_at: u64,
}
