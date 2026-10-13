//! Storefront row model: the unified game row served to the UI by search,
//! trending and caches. Kept separate from the `StoreService` orchestration
//! and the Steam/Hubcap mapping helpers.
use serde::{Deserialize, Serialize};

use crate::game_info::model::{GameInfoPlatforms, GameInfoPrice, GameInfoStoreCategories};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UnifiedStoreGame {
    pub id: u32,
    pub name: String,
    #[serde(rename = "appId")]
    pub app_id: String,
    pub has_manifest: bool,
    pub has_denuvo: bool,
    /// True when Steam content descriptors (or the name heuristic) mark the
    /// title as sexually explicit. The UI uses it for the pink card border;
    /// it stays `false` on rows Steam cannot classify (unknown metadata).
    /// `#[serde(default)]` keeps pre-NSFW search-cache entries deserializable.
    #[serde(default)]
    pub has_nsfw: bool,
    /// True when Steam flags the item as `unlisted` (delisted). The UI uses
    /// it for the white card border. `#[serde(default)]` keeps older
    /// search-cache entries deserializable.
    #[serde(default)]
    pub has_delisted: bool,
    #[serde(rename = "imageUrl")]
    pub image_url: String,
    #[serde(default, rename = "heroImageUrl")]
    pub hero_image_url: String,
    #[serde(default)]
    pub store_kind: String,
    #[serde(default)]
    pub release_date_unix: Option<i64>,
    #[serde(default)]
    pub original_release_date_unix: Option<i64>,
    #[serde(default)]
    pub store_url_path: Option<String>,
    #[serde(default)]
    pub price: Option<GameInfoPrice>,
    #[serde(default)]
    pub metascore: Option<String>,
    #[serde(default)]
    pub controller_support: Option<String>,
    #[serde(default)]
    pub platforms: Option<GameInfoPlatforms>,
    #[serde(default)]
    pub store_categories: Option<GameInfoStoreCategories>,
    #[serde(default)]
    pub content_descriptor_ids: Vec<u32>,
}
