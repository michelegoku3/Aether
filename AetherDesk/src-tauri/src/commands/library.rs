use super::command_steam_path;
use crate::core::paths::LocalAppPaths;
use crate::game_info::cache::GameInfoCache;
use crate::manifest::pins::{
    pins_from_rows, DepotManifestPin, LuaManifestEdit, LuaManifestPins, LuaManifestRow,
};
use crate::steam::compat::SteamCompat;
use crate::core::settings::{cache_version_with_currency, load_settings, steam_country_code_for_currency};
use crate::steam::app_names::SteamAppNameResolver;
use crate::steam::library::{InstalledSteamGame, SteamLibraryScanner};
use crate::steam::store_items;
use crate::util::browser::open_external_url;

fn is_library_capsule_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (lower.contains("library_capsule") || lower.contains("library_600x900"))
        && !lower.contains("hero")
}

#[tauri::command]
pub async fn get_installed_library_games(
    app: tauri::AppHandle,
) -> Result<Vec<InstalledSteamGame>, String> {
    let settings = load_settings(&app);
    // Unconfigured Steam shows an empty Library; a *misconfigured* one is a
    // real error so the UI shows "path is wrong" instead of "0 games found".
    let steam_root = match crate::steam::resolve::resolve_steam_path(&settings.steam_path) {
        Ok(root) => root,
        Err(crate::steam::resolve::SteamPathError::Empty) => return Ok(Vec::new()),
        Err(error) => return Err(error.message(&settings.steam_path)),
    };

    crate::desk_log_info!("library", "Scanning Steam library for Lua games (steam_path='{}')", steam_root.display());
    let store_currency = settings.store_currency.clone();
    // La scansione (appmanifest + stplug-in) è I/O sincrono: fuori dal
    // runtime tokio, altrimenti ogni rescan (anche quelli del watcher)
    // bloccherebbe i task async dell'app.
    let scanner = SteamLibraryScanner::new(steam_root);
    let mut games = tauri::async_runtime::spawn_blocking(move || scanner.scan_installed_games())
        .await
        .map_err(|e| format!("Library scan task failed: {e}"))?;

    // UI-critical path: use persistent cache first; if any app names are missing
    // on first start, resolve them immediately so Library and Home search render
    // with real game names instead of raw App IDs.
    let cache_dir = LocalAppPaths::data_root().join("cache");
    let resolver = SteamAppNameResolver::new(cache_dir.clone());
    let app_ids: Vec<u32> = games.iter().map(|game| game.id).collect();
    let mut names = resolver.cached_names(app_ids.clone());
    if names.len() < app_ids.len() {
        names = resolver.resolve_names(app_ids.clone()).await;
    }
    let mut image_urls = resolver.cached_image_urls(app_ids.clone());
    let mut hero_image_urls = resolver.cached_hero_image_urls(app_ids.clone());

    // First launch / after cache wipe: do not paint guessed CDN paths
    // (library_600x900.jpg, header.jpg, …). Those 404 or look like heroes.
    // appdetails `capsule_image` is a wide store banner — also not a library
    // capsule. The GetItems batch used after restart has the hashed
    // library_capsule; call it now so the first paint is correct.
    let missing_covers: Vec<u32> = app_ids
        .iter()
        .copied()
        .filter(|app_id| {
            image_urls
                .get(app_id)
                .map(|url| !is_library_capsule_url(url))
                .unwrap_or(true)
        })
        .collect();
    if !missing_covers.is_empty() {
        let country = steam_country_code_for_currency(&store_currency);
        crate::desk_log_info!(
            "library",
            "Fetching Steam GetItems capsules for {} uncached App ID(s) (country={})",
            missing_covers.len(),
            country
        );
        let metas = store_items::fetch_store_items_for_country(missing_covers, country).await;
        resolver.merge_image_urls(
            metas
                .iter()
                .filter_map(|(app_id, meta)| meta.library_capsule_url.clone().map(|url| (*app_id, url))),
        );
        resolver.merge_hero_image_urls(
            metas
                .into_iter()
                .filter_map(|(app_id, meta)| meta.hero_image_url.map(|url| (app_id, url))),
        );
        image_urls = resolver.cached_image_urls(app_ids.clone());
        hero_image_urls = resolver.cached_hero_image_urls(app_ids.clone());
    }

    for game in &mut games {
        if let Some(name) = names.get(&game.id) {
            game.name = name.clone();
        }
        if let Some(image_url) = image_urls.get(&game.id).filter(|url| is_library_capsule_url(url)) {
            game.image_url = image_url.clone();
        } else {
            game.image_url.clear();
        }
        if let Some(hero_image_url) = hero_image_urls.get(&game.id) {
            game.hero_image_url = hero_image_url.clone();
        } else {
            game.hero_image_url.clear();
        }
    }

    games.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let info_cache_version = cache_version_with_currency(
        &app.package_info().version.to_string(),
        &store_currency,
    );
    GameInfoCache::new(cache_dir, info_cache_version)
        .merge_library_games(&games);
    crate::desk_log_info!("library", "Library scan completed: returned {} installed Lua game(s)", games.len());
    Ok(games)
}

#[tauri::command]
pub async fn warm_library_game_cache(app: tauri::AppHandle) -> Result<usize, String> {
    let settings = load_settings(&app);
    // Fire-and-forget background warm-up: never fail, but log a misconfigured
    // path instead of silently warming nothing.
    let steam_root = match crate::steam::resolve::resolve_steam_path(&settings.steam_path) {
        Ok(root) => root,
        Err(crate::steam::resolve::SteamPathError::Empty) => return Ok(0),
        Err(error) => {
            crate::desk_log_warn!("library", "Cache warm-up skipped: {}", error.message(&settings.steam_path));
            return Ok(0);
        }
    };

    let scanner = SteamLibraryScanner::new(steam_root);
    let games = scanner.scan_installed_games();
    let app_ids: Vec<u32> = games.iter().map(|game| game.id).collect();

    if app_ids.is_empty() {
        return Ok(0);
    }

    crate::desk_log_info!("library", "Starting background metadata cache warm-up for {} installed game App ID(s)", app_ids.len());
    let cache_dir = LocalAppPaths::data_root().join("cache");
    let resolver = SteamAppNameResolver::new(cache_dir);
    let names = resolver.resolve_names(app_ids.clone()).await;
    let country = steam_country_code_for_currency(&settings.store_currency);
    let metas = store_items::fetch_store_items_for_country(app_ids, country).await;
    let meta_values: Vec<(u32, store_items::StoreItemMeta)> = metas.into_iter().collect();
    resolver.merge_image_urls(
        meta_values
            .iter()
            .filter_map(|(app_id, meta)| meta.library_capsule_url.clone().map(|url| (*app_id, url))),
    );
    resolver.merge_hero_image_urls(
        meta_values
            .into_iter()
            .filter_map(|(app_id, meta)| meta.hero_image_url.map(|url| (app_id, url))),
    );

    crate::desk_log_info!("library", "Background metadata cache warm-up complete: {} name(s) cached", names.len());
    Ok(names.len())
}

#[tauri::command]
pub fn open_steamdb_depots(app_id: u32) -> Result<(), String> {
    if app_id == 0 {
        return Err("A valid Steam App ID is required".to_string());
    }

    crate::desk_log_info!("library", "Opening SteamDB depots page for {}", crate::core::logger::format_appid(app_id));
    let url = format!("https://steamdb.info/app/{}/depots/", app_id);
    open_external_url(&url)
}

/// Open SteamDB patch notes page for the given App ID (used by the Auto tab
/// of the Change Version popup, since the Auto flow is build-driven and the
/// depots page is only useful in the per-depot Manual flow).
#[tauri::command]
pub fn open_steamdb_patchnotes(app_id: u32) -> Result<(), String> {
    if app_id == 0 {
        return Err("A valid Steam App ID is required".to_string());
    }

    crate::desk_log_info!("library", "Opening SteamDB patchnotes page for {}", crate::core::logger::format_appid(app_id));
    let url = format!("https://steamdb.info/app/{}/patchnotes/", app_id);
    open_external_url(&url)
}



#[tauri::command]
pub fn get_installed_lua_manifest_rows(
    app: tauri::AppHandle,
    app_id: u32,
) -> Result<Vec<LuaManifestRow>, String> {
    let steam_path = command_steam_path(&app)?;
    crate::desk_log_debug!("library", "Reading installed Lua manifest rows for {} from '{}'", crate::core::logger::format_appid(app_id), steam_path);
    // Tolerant read: this feeds the version editor, which must open even when
    // the Lua contains a malformed setManifestid line — that line is exactly
    // what the user may be there to fix. Malformed lines come back as rows
    // carrying `issue`, so the editor can mark and repair them; writing is
    // still gated by `validate_content` (F4).
    let pins = LuaManifestPins::new(steam_path, app_id);
    let rows = pins.editor_rows_from_file()?;
    if let Some(issue) = rows.iter().find_map(|row| row.issue.as_ref()) {
        crate::desk_log_warn!(
            "library",
            "Lua for {} contains {} malformed setManifestid line(s); first at line {} ({})",
            crate::core::logger::format_appid(app_id),
            rows.iter().filter(|row| row.issue.is_some()).count(),
            issue.line,
            issue.problem.reason()
        );
    }
    Ok(rows)
}

/// Last backend Library invalidation revision.
///
/// The React provider primarily receives pushed `library://lua-changed` events.
/// It reads this atomic value only as a low-cost recovery path for a WebView
/// that misses a native push; this command never scans Steam or the filesystem.
#[tauri::command]
pub fn get_library_change_revision(app: tauri::AppHandle) -> u64 {
    crate::core::library_events::current_library_change_revision(&app)
}

#[tauri::command]
pub fn get_lua_game_update_state(app: tauri::AppHandle, app_id: u32) -> Result<bool, String> {
    let steam_path = command_steam_path(&app)?;
    crate::desk_log_debug!("library", "Checking update state for {} in '{}'", crate::core::logger::format_appid(app_id), steam_path);
    LuaManifestPins::new(steam_path, app_id).updates_are_enabled()
}

#[tauri::command]
pub fn set_lua_game_updates_enabled(
    app: tauri::AppHandle,
    app_id: u32,
    enabled: bool,
) -> Result<String, String> {
    let steam_path = command_steam_path(&app)?;
    crate::desk_log_info!("library", "Setting updates_enabled={} for {} in steam_path='{}'", enabled, crate::core::logger::format_appid(app_id), steam_path);
    // Safety net before locking a game to a fixed version: realign the
    // informational pins to the manifests Steam ACTUALLY installed, so
    // "Disable updates" reactivates the CURRENT version's GIDs. Without this,
    // updates Steam applied while the synchronizer could not observe them
    // (Desk closed, checkpoint reset, cleaned depotcache) would resurrect a
    // stale pin and trigger a downgrade. Local-only and best-effort: a
    // failure here must not block the user's explicit toggle.
    if !enabled {
        match crate::core::hubcap_update_monitor::realign_pins_to_installed(&steam_path, app_id) {
            Ok(0) => {}
            Ok(realigned) => crate::desk_log_info!(
                "library",
                "Pre-lock pin realignment for {}: {} pin(s) moved to the installed manifests",
                crate::core::logger::format_appid(app_id),
                realigned
            ),
            Err(error) => crate::desk_log_warn!(
                "library",
                "Pre-lock pin realignment failed for {}: {} (continuing with the toggle)",
                crate::core::logger::format_appid(app_id),
                error
            ),
        }
    }
    let changed = match LuaManifestPins::new(steam_path.clone(), app_id).set_updates_enabled(enabled) {
        Ok(c) => c,
        Err(e) => {
            crate::desk_log_error!("library", "Failed to set updates_enabled={} for {}: {}", enabled, crate::core::logger::format_appid(app_id), e);
            return Err(e);
        }
    };
    crate::desk_log_info!("library", "Updates {} for {}: {} manifest pin(s) modified", if enabled { "enabled" } else { "disabled" }, crate::core::logger::format_appid(app_id), changed);
    // Every Lua change must land in AetherData: the toggle rewrote the pins
    // (comment/uncomment), so archive the resulting Lua version. Deduplicated
    // by content — repeated toggles never create duplicate copies. For the
    // disable direction the pre-lock realignment above already archived its
    // own pre/post states; this captures the final uncommented-pins version.
    if changed > 0 {
        if let (Ok(final_lua), Ok(backup)) = (
            LuaManifestPins::new(steam_path.clone(), app_id).read_lua(),
            crate::core::backup::GameBackup::for_app(app_id),
        ) {
            let _ = backup.store_history_version(app_id, final_lua.as_bytes());
        }
    }

    let message = if enabled {
        format!(
            "Updates enabled for App ID {}. {} manifest pin(s) disabled.",
            app_id, changed
        )
    } else {
        format!(
            "Updates disabled for App ID {}. {} manifest pin(s) restored.",
            app_id, changed
        )
    };
    crate::core::library_events::notify_lua_changed(
        &app,
        crate::core::library_events::LibraryChangeOrigin::LibraryAction,
        [app_id],
    );
    Ok(message)
}

#[tauri::command]
pub fn remove_lua_game_from_library(
    app: tauri::AppHandle,
    app_id: u32,
) -> Result<String, String> {
    let steam_path = command_steam_path(&app)?;
    crate::desk_log_info!("library", "Removing Lua game {} from library (steam_path='{}')", crate::core::logger::format_appid(app_id), steam_path);

    // `command_steam_path` above already strict-validated and normalized the
    // path; the scanner normalizes again, so the value is safe to pass.
    let scanner = SteamLibraryScanner::new(steam_path.clone());
    if scanner.is_app_installed(app_id) {
        crate::desk_log_warn!("library", "Cannot remove {}: game is currently installed in Steam", crate::core::logger::format_appid(app_id));
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
        crate::desk_log_info!("library", "Successfully removed Lua files for {} from stplug-in", crate::core::logger::format_appid(app_id));
        crate::core::library_events::notify_lua_changed(
            &app,
            crate::core::library_events::LibraryChangeOrigin::LibraryAction,
            [app_id],
        );
        Ok(format!("App ID {} removed from Aether library.", app_id))
    } else {
        crate::desk_log_warn!("library", "No Lua file found for {} in stplug-in", crate::core::logger::format_appid(app_id));
        Ok(format!("No Lua file found for App ID {}.", app_id))
    }
}

#[tauri::command]
pub async fn apply_specific_version_edits(
    app: tauri::AppHandle,
    app_id: u32,
    edits: Vec<LuaManifestEdit>,
) -> Result<Vec<LuaManifestRow>, String> {
    let steam_path = command_steam_path(&app)?;
    crate::desk_log_info!(
        "library",
        "Applying {} specific version edit(s) for {} in steam_path='{}'",
        edits.len(),
        crate::core::logger::format_appid(app_id),
        steam_path
    );

    let pins_editor = LuaManifestPins::new(steam_path.clone(), app_id);
    let (next_lua, rows) = pins_editor.preview_edits(&edits)?;
    let pins: Vec<DepotManifestPin> = pins_from_rows(LuaManifestPins::rows_for_manifest_sync(
        &next_lua,
    ));

    // Local-first resolution (shared with every other pipeline): backup and
    // secondary-cache hits are restored into depotcache first; only genuinely
    // absent manifests are generated through the configured Hubcap key.
    let settings = load_settings(&app);
    let generation = if settings.hubcap_api_key.trim().is_empty() {
        crate::manifest::resolver::Generation::LocalOnly
    } else {
        crate::manifest::resolver::Generation::SettingsKey(settings.hubcap_api_key.clone())
    };
    let resolution = crate::manifest::resolver::resolve(
        crate::manifest::resolver::ManifestRequest {
            steam_path: steam_path.clone(),
            app_id,
            pins: pins.clone(),
            generation,
        },
    )
    .await?;
    if !resolution.is_complete() {
        return Err(
            "The selected version references manifest files that are not local. Configure a valid authenticated Hubcap API key before applying it.".to_string(),
        );
    }
    let generated = resolution.generated;

    // Publish the Lua and any newly generated exact manifests together. The
    // old Lua remains untouched if staging/generation fails.
    SteamCompat::new(steam_path.clone())
        .install_lua_and_manifest_files(app_id, &next_lua, &generated)?;

    // Shared completeness gate: the same "present" definition the resolution
    // used, so a resolved pin can never fail here.
    let remaining = crate::manifest::resolver::verify_available(&steam_path, app_id, &pins);
    if !remaining.missing.is_empty() {
        return Err(format!(
            "Specific version was not committed safely: {} manifest(s) are still missing",
            remaining.missing.len()
        ));
    }

    if let Ok(backup) = crate::core::backup::GameBackup::for_app(app_id) {
        let depotcache = std::path::PathBuf::from(&steam_path).join("depotcache");
        let _ = backup.backup_lua_artifacts(app_id, &next_lua, &generated);
        let _ = backup.backup_referenced_manifests(&next_lua, &depotcache);
    }
    crate::core::library_events::notify_lua_changed(
        &app,
        crate::core::library_events::LibraryChangeOrigin::LibraryAction,
        [app_id],
    );
    crate::desk_log_info!(
        "library",
        "Successfully applied specific version edits for {}: {} row(s) active, {} manifest(s) generated",
        crate::core::logger::format_appid(app_id),
        rows.len(),
        generated.len()
    );
    Ok(rows)
}


