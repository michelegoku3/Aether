//! Store application workflows.
//!
//! Public entry points are called by `commands::store`. This module owns store
//! search orchestration and provider-package installation, while provider,
//! manifest and Steam details remain in their respective adapters.
//!
//! Invariants:
//! - a game mutation is committed immediately before Steam-side writes;
//! - every enabled manifest pin is available before an installed Lua is
//!   treated as complete;
//! - manifests are published before the Lua hot-reload trigger;
//! - command names and IPC payloads are owned by the command adapter.

mod concurrency;
mod installation;
mod latest_download;
mod model;
mod package_completion;
mod search;
mod source;
mod specific_version;
mod validation;

pub use latest_download::{
    trigger_hubcap_download, trigger_luatools_download, trigger_ryuu_download,
};
pub use specific_version::{
    prepare_luatools_specific_version_download, prepare_ryuu_specific_version_download,
    prepare_specific_version_download,
};
pub use model::{
    AuthenticatedDownloadRequest, CachedStoreSearchResponse, DownloadRequest,
    LuaToolsDownloadRequest, StoreSuggestItem,
};
pub use search::{
    check_denuvo_bulk, get_cached_store_search, get_trending_store_games, search_store,
    suggest_store_games,
};
