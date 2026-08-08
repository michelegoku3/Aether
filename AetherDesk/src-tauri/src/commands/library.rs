use crate::core::paths::LocalAppPaths;
use crate::game_info::cache::GameInfoCache;
use crate::manifest::pins::{LuaManifestEdit, LuaManifestPins, LuaManifestRow};
use crate::core::settings::{cache_version_with_currency, SettingsManager};
use crate::steam::app_names::SteamAppNameResolver;
use crate::steam::library::{InstalledSteamGame, SteamLibraryScanner};
use crate::steam::store_items;
use crate::util::validation::validate_steam_path;
use crate::util::browser::open_external_url;

#[tauri::command]
pub async fn get_installed_library_games(
    app: tauri::AppHandle,
) -> Result<Vec<InstalledSteamGame>, String> {
    let settings = SettingsManager::new(&app).load();
    if settings.steam_path.trim().is_empty() {
        return Ok(Vec::new());
    }

    let store_currency = settings.store_currency.clone();
    let scanner = SteamLibraryScanner::new(settings.steam_path, Some(settings.active_library));
    let mut games = scanner.scan_installed_games();

    // UI-critical path: use persistent cache only, never wait for Steam/network here.
    let cache_dir = LocalAppPaths::data_root().join("cache");
    let resolver = SteamAppNameResolver::new(cache_dir.clone());
    let app_ids: Vec<u32> = games.iter().map(|game| game.id).collect();
    let names = resolver.cached_names(app_ids.clone());
    let image_urls = resolver.cached_image_urls(app_ids);

    for game in &mut games {
        if let Some(name) = names.get(&game.id) {
            game.name = name.clone();
        }
        if let Some(image_url) = image_urls.get(&game.id) {
            game.image_url = image_url.clone();
        }
    }

    games.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let info_cache_version = cache_version_with_currency(
        &app.package_info().version.to_string(),
        &store_currency,
    );
    GameInfoCache::new(cache_dir, info_cache_version)
        .merge_library_games(&games);
    Ok(games)
}

#[tauri::command]
pub async fn warm_library_game_cache(app: tauri::AppHandle) -> Result<usize, String> {
    let settings = SettingsManager::new(&app).load();
    if settings.steam_path.trim().is_empty() {
        return Ok(0);
    }

    let scanner = SteamLibraryScanner::new(settings.steam_path, Some(settings.active_library));
    let games = scanner.scan_installed_games();
    let app_ids: Vec<u32> = games.iter().map(|game| game.id).collect();

    if app_ids.is_empty() {
        return Ok(0);
    }

    let cache_dir = LocalAppPaths::data_root().join("cache");
    let resolver = SteamAppNameResolver::new(cache_dir);
    let names = resolver.resolve_names(app_ids.clone()).await;
    let metas = store_items::fetch_store_items_for_country(app_ids, "US").await;
    resolver.merge_image_urls(
        metas
            .into_iter()
            .filter_map(|(app_id, meta)| meta.library_capsule_url.map(|url| (app_id, url))),
    );

    Ok(names.len())
}

#[tauri::command]
pub fn open_steamdb_depots(app_id: u32) -> Result<(), String> {
    if app_id == 0 {
        return Err("A valid Steam App ID is required".to_string());
    }

    let url = format!("https://steamdb.info/app/{}/depots/", app_id);
    open_external_url(&url)
}



#[tauri::command]
pub fn get_installed_lua_manifest_rows(
    app_id: u32,
    steam_path: String,
) -> Result<Vec<LuaManifestRow>, String> {
    validate_steam_path(&steam_path)?;
    LuaManifestPins::new(steam_path, app_id).rows_from_file()
}

#[tauri::command]
pub fn get_lua_game_update_state(app_id: u32, steam_path: String) -> Result<bool, String> {
    validate_steam_path(&steam_path)?;
    LuaManifestPins::new(steam_path, app_id).updates_are_enabled()
}

#[tauri::command]
pub fn set_lua_game_updates_enabled(
    app_id: u32,
    steam_path: String,
    enabled: bool,
) -> Result<String, String> {
    validate_steam_path(&steam_path)?;
    let changed = LuaManifestPins::new(steam_path, app_id).set_updates_enabled(enabled)?;

    if enabled {
        Ok(format!(
            "Updates enabled for App ID {}. {} manifest pin(s) disabled.",
            app_id, changed
        ))
    } else {
        Ok(format!(
            "Updates disabled for App ID {}. {} manifest pin(s) restored.",
            app_id, changed
        ))
    }
}

#[tauri::command]
pub fn remove_lua_game_from_library(
    app: tauri::AppHandle,
    app_id: u32,
    steam_path: String,
) -> Result<String, String> {
    validate_steam_path(&steam_path)?;

    let settings = SettingsManager::new(&app).load();
    let scanner = SteamLibraryScanner::new(steam_path.clone(), Some(settings.active_library));
    if scanner.is_app_installed(app_id) {
        return Err("This game is installed in Steam. Remove is allowed only for Lua-only games that are not installed.".to_string());
    }

    let plugin_dir = std::path::PathBuf::from(steam_path)
        .join("config")
        .join("stplug-in");
    let lua_path = plugin_dir.join(format!("{}.lua", app_id));
    let backup_path = plugin_dir.join(format!("{}.lua.bak", app_id));

    let mut removed = false;
    if lua_path.exists() {
        std::fs::remove_file(&lua_path)
            .map_err(|e| format!("Failed to remove Lua file {}: {}", lua_path.display(), e))?;
        removed = true;
    }
    if backup_path.exists() {
        let _ = std::fs::remove_file(&backup_path);
    }

    if removed {
        Ok(format!("App ID {} removed from Aether library.", app_id))
    } else {
        Ok(format!("No Lua file found for App ID {}.", app_id))
    }
}

#[tauri::command]
pub fn apply_specific_version_edits(
    app_id: u32,
    steam_path: String,
    edits: Vec<LuaManifestEdit>,
) -> Result<Vec<LuaManifestRow>, String> {
    validate_steam_path(&steam_path)?;
    LuaManifestPins::new(steam_path, app_id).apply_edits(edits)
}


