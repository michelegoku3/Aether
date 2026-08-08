use crate::core::paths::LocalAppPaths;
use crate::core::settings::SettingsManager;
use crate::game_info::cache::{GameInfoCache, GAME_INFO_TTL_SECONDS};
use crate::game_info::model::{
    GameInfo, GameInfoAppDetails, GameInfoLocal, GameInfoPlatforms, GameInfoPrice,
    GameInfoStoreCategories,
};
use crate::manifest::pins::LuaManifestPins;
use crate::providers::{http, hubcap::HubcapClient};
use crate::steam::{api, library::SteamLibraryScanner, store_items};

const APPDETAILS_TIMEOUT_SECONDS: u64 = 6;
const HUBCAP_INFO_TTL_SECONDS: u64 = 24 * 60 * 60;

pub struct GameInfoService {
    app: tauri::AppHandle,
    cache: GameInfoCache,
}

impl GameInfoService {
    pub fn new(app: tauri::AppHandle) -> Self {
        let cache = GameInfoCache::new(
            LocalAppPaths::data_root().join("cache"),
            app.package_info().version.to_string(),
        );
        Self { app, cache }
    }

    pub async fn get_game_info(&self, app_id: u32) -> Result<GameInfo, String> {
        if app_id == 0 {
            return Err("A valid Steam App ID is required.".to_string());
        }

        let mut info = self.cache.get(app_id).unwrap_or_else(|| GameInfo::new(app_id));

        self.merge_local_info(&mut info);

        if !GameInfoCache::is_fresh(info.store_items_updated_at_unix, GAME_INFO_TTL_SECONDS) {
            self.merge_store_items_info(&mut info).await;
        }

        if !GameInfoCache::is_fresh(info.appdetails_updated_at_unix, GAME_INFO_TTL_SECONDS) {
            self.merge_appdetails_info(&mut info).await;
        }

        if !GameInfoCache::is_fresh(info.hubcap_updated_at_unix, HUBCAP_INFO_TTL_SECONDS) {
            self.merge_hubcap_info(&mut info).await;
        }

        let _ = self.cache.put(info.clone());
        Ok(info)
    }

    fn merge_local_info(&self, info: &mut GameInfo) {
        let settings = SettingsManager::new(&self.app).load();
        if settings.steam_path.trim().is_empty() {
            return;
        }

        let scanner = SteamLibraryScanner::new(settings.steam_path.clone(), Some(settings.active_library.clone()));
        let Some(game) = scanner
            .scan_installed_games()
            .into_iter()
            .find(|game| game.id == info.app_id)
        else {
            return;
        };

        if info.name.as_ref().map(|name| name.trim().is_empty()).unwrap_or(true) {
            info.name = Some(game.name.clone());
        }
        if info.image_url.as_ref().map(|url| url.trim().is_empty()).unwrap_or(true) {
            info.image_url = Some(game.image_url.clone());
        }
        info.store_url = Some(format!("https://store.steampowered.com/app/{}/", info.app_id));

        let rows = LuaManifestPins::new(settings.steam_path.clone(), info.app_id)
            .rows_from_file()
            .unwrap_or_default();
        let updates_enabled = LuaManifestPins::new(settings.steam_path, info.app_id)
            .updates_are_enabled()
            .ok();

        info.local = Some(GameInfoLocal {
            installed: game.installed,
            install_dir: non_empty_string(&game.install_dir),
            library_path: non_empty_string(&game.library_path),
            game_path: non_empty_string(&game.game_path),
            lua_installed: true,
            manifest_pin_count: rows.len(),
            updates_enabled,
        });
        let now = GameInfoCache::now_unix();
        info.updated_at_unix = now;
        info.local_updated_at_unix = Some(now);
    }

    async fn merge_store_items_info(&self, info: &mut GameInfo) {
        let meta_map = store_items::fetch_store_items(vec![info.app_id]).await;
        let Some(meta) = meta_map.get(&info.app_id) else {
            return;
        };

        if !meta.kind.is_empty() {
            info.kind = Some(meta.kind.clone());
        }
        info.has_nsfw = Some(store_items::is_nsfw(meta, info.name.as_deref().unwrap_or_default()));
        info.has_delisted = Some(meta.is_delisted);
        info.release_date_unix = meta.release_date_unix.or(info.release_date_unix);
        info.original_release_date_unix = meta.original_release_date_unix.or(info.original_release_date_unix);
        info.store_url_path = meta.store_url_path.clone().or_else(|| info.store_url_path.clone());
        info.platforms = Some(platforms_from_store_meta(meta, info.platforms.clone()));
        info.store_categories = Some(GameInfoStoreCategories {
            supported_player_category_ids: meta.categories.supported_player_category_ids.clone(),
            feature_category_ids: meta.categories.feature_category_ids.clone(),
            controller_category_ids: meta.categories.controller_category_ids.clone(),
        });
        info.content_descriptor_ids = meta.content_descriptor_ids.clone();
        if info.price.is_none() {
            info.price = meta.best_purchase_option.as_ref().map(|price| GameInfoPrice {
                currency: None,
                initial_cents: None,
                final_cents: price
                    .final_price_in_cents
                    .as_ref()
                    .and_then(|value| value.parse::<i64>().ok()),
                formatted_final: price.formatted_final_price.clone(),
                discount_percent: None,
            });
        }

        let now = GameInfoCache::now_unix();
        info.updated_at_unix = now;
        info.store_items_updated_at_unix = Some(now);
    }

    async fn merge_hubcap_info(&self, info: &mut GameInfo) {
        let settings = SettingsManager::new(&self.app).load();
        if settings.hubcap_api_key.trim().is_empty() {
            return;
        }

        let client = HubcapClient::new(settings.hubcap_api_key);
        let has_manifest = client.has_manifest(info.app_id).await;
        info.has_manifest = Some(has_manifest);

        let now = GameInfoCache::now_unix();
        info.updated_at_unix = now;
        info.hubcap_updated_at_unix = Some(now);
    }

    async fn merge_appdetails_info(&self, info: &mut GameInfo) {
        let client = http::build_client(APPDETAILS_TIMEOUT_SECONDS);
        let Ok(envelope) = api::fetch_app_details(&client, info.app_id).await else {
            return;
        };
        let Some(data) = envelope.data else {
            return;
        };

        merge_appdetails_value(info, &data);
        let now = GameInfoCache::now_unix();
        info.updated_at_unix = now;
        info.appdetails_updated_at_unix = Some(now);
    }
}

fn merge_appdetails_value(info: &mut GameInfo, data: &serde_json::Value) {
    if info.name.as_ref().map(|name| name.trim().is_empty()).unwrap_or(true) {
        info.name = string_field(data, "name");
    }
    info.store_url = Some(format!("https://store.steampowered.com/app/{}/", info.app_id));

    if info.image_url.as_ref().map(|url| url.trim().is_empty()).unwrap_or(true) {
        info.image_url = string_field(data, "capsule_imagev5")
            .or_else(|| string_field(data, "capsule_image"))
            .or_else(|| string_field(data, "header_image"));
    }

    if let Some(platforms) = data.get("platforms") {
        let mut current = info.platforms.clone().unwrap_or_default();
        current.windows = bool_field(platforms, "windows").or(current.windows);
        current.mac = bool_field(platforms, "mac").or(current.mac);
        current.linux = bool_field(platforms, "linux").or(current.linux);
        info.platforms = Some(current);
    }

    if let Some(price) = data.get("price_overview") {
        info.price = Some(GameInfoPrice {
            currency: string_field(price, "currency"),
            initial_cents: i64_field(price, "initial"),
            final_cents: i64_field(price, "final"),
            formatted_final: string_field(price, "final_formatted"),
            discount_percent: i64_field(price, "discount_percent"),
        });
    }

    if let Some(metacritic) = data.get("metacritic") {
        info.metascore = u64_field(metacritic, "score").map(|score| score.to_string()).or_else(|| info.metascore.clone());
    }

    let drm_notice = string_field(data, "drm_notice");
    if let Some(notice) = drm_notice.as_ref() {
        info.has_denuvo = Some(notice.to_lowercase().contains("denuvo"));
    }

    let app_details = GameInfoAppDetails {
        required_age: value_to_string(data.get("required_age")),
        is_free: bool_field(data, "is_free"),
        short_description: string_field(data, "short_description"),
        supported_languages: string_field(data, "supported_languages"),
        website: string_field(data, "website"),
        header_image: string_field(data, "header_image"),
        capsule_image: string_field(data, "capsule_image"),
        background: string_field(data, "background"),
        developers: string_array_field(data, "developers"),
        publishers: string_array_field(data, "publishers"),
        genres: description_array_field(data, "genres"),
        categories: description_array_field(data, "categories"),
        recommendations_total: data.get("recommendations").and_then(|v| u64_field(v, "total")),
        achievements_total: data.get("achievements").and_then(|v| u64_field(v, "total")),
        metacritic_score: data.get("metacritic").and_then(|v| u64_field(v, "score")),
        release_date_text: data.get("release_date").and_then(|v| string_field(v, "date")),
        coming_soon: data.get("release_date").and_then(|v| bool_field(v, "coming_soon")),
        drm_notice,
    };
    info.app_details = Some(app_details);
}

fn platforms_from_store_meta(
    meta: &store_items::StoreItemMeta,
    existing: Option<GameInfoPlatforms>,
) -> GameInfoPlatforms {
    let mut platforms = existing.unwrap_or_default();
    platforms.windows = meta.platforms.windows.or(platforms.windows);
    platforms.mac = meta.platforms.mac.or(platforms.mac);
    platforms.linux = meta.platforms.linux.or(platforms.linux);
    platforms.steam_deck_compat_category = meta
        .platforms
        .steam_deck_compat_category
        .or(platforms.steam_deck_compat_category);
    platforms.steam_os_compat_category = meta
        .platforms
        .steam_os_compat_category
        .or(platforms.steam_os_compat_category);
    platforms.steam_machine_compat_category = meta
        .platforms
        .steam_machine_compat_category
        .or(platforms.steam_machine_compat_category);
    platforms.has_vr_support = meta.platforms.has_vr_support.or(platforms.has_vr_support);
    platforms
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn string_array_field(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn description_array_field(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| string_field(item, "description"))
                .collect()
        })
        .unwrap_or_default()
}

fn bool_field(value: &serde_json::Value, key: &str) -> Option<bool> {
    value.get(key).and_then(|v| v.as_bool())
}

fn i64_field(value: &serde_json::Value, key: &str) -> Option<i64> {
    value.get(key).and_then(|v| v.as_i64())
}

fn u64_field(value: &serde_json::Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|v| v.as_u64())
}

fn value_to_string(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn non_empty_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
