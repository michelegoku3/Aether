use crate::game_info::model::{GameInfo, GameInfoLocal};
use crate::steam::library::InstalledSteamGame;
use crate::store::service::UnifiedStoreGame;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const CACHE_FILE_NAME: &str = "game_info_cache.json";
pub const GAME_INFO_TTL_SECONDS: u64 = 14 * 24 * 60 * 60;

#[derive(Debug, Default, Serialize, Deserialize)]
struct GameInfoCacheFile {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    app_version: String,
    #[serde(default)]
    entries: HashMap<u32, GameInfo>,
}

pub struct GameInfoCache {
    cache_path: PathBuf,
    app_version: String,
}

impl GameInfoCache {
    pub fn new(cache_dir: PathBuf, app_version: String) -> Self {
        Self {
            cache_path: cache_dir.join(CACHE_FILE_NAME),
            app_version,
        }
    }

    pub fn get(&self, app_id: u32) -> Option<GameInfo> {
        self.load_cache().entries.get(&app_id).cloned()
    }

    pub fn put(&self, info: GameInfo) -> Result<(), String> {
        if info.app_id == 0 {
            return Ok(());
        }
        let mut cache = self.load_cache();
        cache.entries.insert(info.app_id, info);
        self.save_cache(&cache)
    }

    pub fn merge_store_results(&self, games: &[UnifiedStoreGame]) {
        if games.is_empty() {
            return;
        }

        let now = Self::now_unix();
        let mut cache = self.load_cache();
        let mut changed = false;

        for game in games {
            if game.id == 0 {
                continue;
            }
            let entry = cache
                .entries
                .entry(game.id)
                .or_insert_with(|| GameInfo::new(game.id));

            merge_non_empty(&mut entry.name, &game.name);
            merge_non_empty(&mut entry.image_url, &game.image_url);
            entry.store_url = Some(format!("https://store.steampowered.com/app/{}/", game.id));
            entry.has_manifest = Some(game.has_manifest);
            entry.has_denuvo = Some(game.has_denuvo);
            entry.has_nsfw = Some(game.has_nsfw);
            entry.has_delisted = Some(game.has_delisted);

            merge_non_empty(&mut entry.kind, &game.store_kind);
            entry.release_date_unix = game.release_date_unix.or(entry.release_date_unix);
            entry.original_release_date_unix = game.original_release_date_unix.or(entry.original_release_date_unix);
            entry.store_url_path = game.store_url_path.clone().or_else(|| entry.store_url_path.clone());
            entry.price = game.price.clone().or_else(|| entry.price.clone());
            entry.metascore = game.metascore.clone().or_else(|| entry.metascore.clone());
            entry.controller_support = game.controller_support.clone().or_else(|| entry.controller_support.clone());
            entry.platforms = game.platforms.clone().or_else(|| entry.platforms.clone());
            entry.store_categories = game.store_categories.clone().or_else(|| entry.store_categories.clone());
            if !game.content_descriptor_ids.is_empty() {
                entry.content_descriptor_ids = game.content_descriptor_ids.clone();
            }

            entry.updated_at_unix = now;
            entry.store_search_updated_at_unix = Some(now);
            if game.release_date_unix.is_some() || !game.store_kind.trim().is_empty() {
                entry.store_items_updated_at_unix = Some(now);
            }
            if game.has_manifest {
                entry.hubcap_updated_at_unix = Some(now);
            }
            changed = true;
        }

        if changed {
            let _ = self.save_cache(&cache);
        }
    }

    pub fn merge_library_games(&self, games: &[InstalledSteamGame]) {
        if games.is_empty() {
            return;
        }

        let now = Self::now_unix();
        let mut cache = self.load_cache();
        let mut changed = false;

        for game in games {
            if game.id == 0 {
                continue;
            }
            let entry = cache
                .entries
                .entry(game.id)
                .or_insert_with(|| GameInfo::new(game.id));

            merge_non_empty(&mut entry.name, &game.name);
            merge_non_empty(&mut entry.image_url, &game.image_url);
            entry.store_url = Some(format!("https://store.steampowered.com/app/{}/", game.id));
            let previous_manifest_pin_count = entry
                .local
                .as_ref()
                .map(|local| local.manifest_pin_count)
                .unwrap_or(0);
            let previous_updates_enabled = entry.local.as_ref().and_then(|local| local.updates_enabled);
            entry.local = Some(GameInfoLocal {
                installed: game.installed,
                install_dir: non_empty_string(&game.install_dir),
                library_path: non_empty_string(&game.library_path),
                game_path: non_empty_string(&game.game_path),
                lua_installed: true,
                manifest_pin_count: previous_manifest_pin_count,
                updates_enabled: previous_updates_enabled,
            });
            entry.updated_at_unix = now;
            entry.local_updated_at_unix = Some(now);
            changed = true;
        }

        if changed {
            let _ = self.save_cache(&cache);
        }
    }

    pub fn merge_denuvo_flags(&self, flags: &HashMap<u32, bool>) {
        if flags.is_empty() {
            return;
        }

        let now = Self::now_unix();
        let mut cache = self.load_cache();
        let mut changed = false;

        for (app_id, has_denuvo) in flags {
            if *app_id == 0 {
                continue;
            }
            let entry = cache
                .entries
                .entry(*app_id)
                .or_insert_with(|| GameInfo::new(*app_id));
            entry.has_denuvo = Some(*has_denuvo);
            entry.updated_at_unix = now;
            changed = true;
        }

        if changed {
            let _ = self.save_cache(&cache);
        }
    }

    pub fn is_fresh(updated_at_unix: Option<u64>, ttl_seconds: u64) -> bool {
        let Some(updated_at) = updated_at_unix else {
            return false;
        };
        Self::now_unix().saturating_sub(updated_at) <= ttl_seconds
    }

    pub fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default()
    }

    fn load_cache(&self) -> GameInfoCacheFile {
        let Ok(content) = fs::read_to_string(&self.cache_path) else {
            return GameInfoCacheFile {
                schema_version: 1,
                app_version: self.app_version.clone(),
                entries: HashMap::new(),
            };
        };

        let mut cache = serde_json::from_str::<GameInfoCacheFile>(&content).unwrap_or_default();
        if cache.schema_version != 1 || cache.app_version != self.app_version {
            cache.entries.clear();
            cache.schema_version = 1;
            cache.app_version = self.app_version.clone();
        }
        cache
    }

    fn save_cache(&self, cache: &GameInfoCacheFile) -> Result<(), String> {
        if let Some(parent) = self.cache_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create game info cache directory: {}", e))?;
        }

        let temp_path = self.cache_path.with_extension("tmp");
        let stamped = GameInfoCacheFile {
            schema_version: 1,
            app_version: self.app_version.clone(),
            entries: cache.entries.clone(),
        };
        let content = serde_json::to_string_pretty(&stamped)
            .map_err(|e| format!("Failed to serialize game info cache: {}", e))?;

        fs::write(&temp_path, content)
            .map_err(|e| format!("Failed to write game info cache: {}", e))?;
        fs::rename(&temp_path, &self.cache_path)
            .map_err(|e| format!("Failed to apply game info cache: {}", e))
    }
}

fn merge_non_empty(target: &mut Option<String>, value: &str) {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        *target = Some(trimmed.to_string());
    }
}

fn non_empty_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
