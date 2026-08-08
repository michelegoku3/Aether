use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfo {
    pub app_id: u32,
    pub name: Option<String>,
    pub image_url: Option<String>,
    pub store_url: Option<String>,
    pub store_url_path: Option<String>,

    pub kind: Option<String>,
    pub has_manifest: Option<bool>,
    pub has_denuvo: Option<bool>,
    pub has_nsfw: Option<bool>,
    pub has_delisted: Option<bool>,

    pub release_date_unix: Option<i64>,
    pub original_release_date_unix: Option<i64>,
    pub price: Option<GameInfoPrice>,
    pub metascore: Option<String>,
    pub controller_support: Option<String>,
    pub platforms: Option<GameInfoPlatforms>,
    pub store_categories: Option<GameInfoStoreCategories>,
    #[serde(default)]
    pub content_descriptor_ids: Vec<u32>,

    pub app_details: Option<GameInfoAppDetails>,
    pub local: Option<GameInfoLocal>,

    pub updated_at_unix: u64,
    pub store_search_updated_at_unix: Option<u64>,
    pub store_items_updated_at_unix: Option<u64>,
    pub appdetails_updated_at_unix: Option<u64>,
    pub hubcap_updated_at_unix: Option<u64>,
    pub local_updated_at_unix: Option<u64>,
}

impl GameInfo {
    pub fn new(app_id: u32) -> Self {
        Self {
            app_id,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfoPrice {
    pub currency: Option<String>,
    pub initial_cents: Option<i64>,
    pub final_cents: Option<i64>,
    pub formatted_final: Option<String>,
    pub discount_percent: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfoPlatforms {
    pub windows: Option<bool>,
    pub mac: Option<bool>,
    pub linux: Option<bool>,
    pub steam_deck_compat_category: Option<u32>,
    pub steam_os_compat_category: Option<u32>,
    pub steam_machine_compat_category: Option<u32>,
    pub has_vr_support: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfoStoreCategories {
    #[serde(default)]
    pub supported_player_category_ids: Vec<u32>,
    #[serde(default)]
    pub feature_category_ids: Vec<u32>,
    #[serde(default)]
    pub controller_category_ids: Vec<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfoAppDetails {
    pub required_age: Option<String>,
    pub is_free: Option<bool>,
    pub short_description: Option<String>,
    pub supported_languages: Option<String>,
    pub website: Option<String>,
    pub header_image: Option<String>,
    pub capsule_image: Option<String>,
    pub background: Option<String>,
    #[serde(default)]
    pub developers: Vec<String>,
    #[serde(default)]
    pub publishers: Vec<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    pub recommendations_total: Option<u64>,
    pub achievements_total: Option<u64>,
    pub metacritic_score: Option<u64>,
    pub release_date_text: Option<String>,
    pub coming_soon: Option<bool>,
    pub drm_notice: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfoLocal {
    pub installed: bool,
    pub install_dir: Option<String>,
    pub library_path: Option<String>,
    pub game_path: Option<String>,
    pub lua_installed: bool,
    pub manifest_pin_count: usize,
    pub updates_enabled: Option<bool>,
}
