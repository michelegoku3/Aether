use crate::store::service::UnifiedStoreGame;
use serde::Serialize;

/// Store-search cache response exposed through the Tauri boundary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedStoreSearchResponse {
    pub results: Vec<UnifiedStoreGame>,
    /// `fresh` = 24h cache hit; `stale` = immediate 14-day fallback that the
    /// frontend should refresh in the background; `miss` = no usable cache.
    pub cache_state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreSuggestItem {
    pub id: u32,
    pub name: String,
    pub app_id: String,
}

/// Provider-independent identity and Steam destination of a store download.
#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub app_id: u32,
    pub steam_path: String,
}

/// Download request for providers authenticated by an API key.
#[derive(Debug, Clone)]
pub struct AuthenticatedDownloadRequest {
    pub download: DownloadRequest,
    pub api_key: String,
}

/// Download request for the session-authenticated LuaTools provider.
#[derive(Debug, Clone)]
pub struct LuaToolsDownloadRequest {
    pub download: DownloadRequest,
    pub game_name: Option<String>,
}
