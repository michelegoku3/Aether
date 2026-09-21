use super::command_steam_path;
use crate::core::backup::GameBackup;
use crate::game_info::cache::GameInfoCache;
use crate::store::drm::DrmDetector;
use crate::providers::hubcap::HubcapClient;
use crate::providers::luatools::LuaToolsClient;
use crate::providers::ryuu::RyuuClient;
use crate::core::paths::LocalAppPaths;
use crate::manifest::pins::{pins_from_rows, DepotManifestPin, LuaManifestPins, LuaManifestRow};
use crate::manifest::package::{ManifestPackage, ManifestPackageFile};
use crate::manifest::resolver;
use crate::core::settings::{cache_version_with_currency, load_settings, normalize_store_currency, normalize_store_front_filter, steam_country_code_for_currency};
use crate::steam::app_names::SteamAppNameResolver;
use crate::steam::compat::SteamCompat;
use crate::store::cache::StoreSearchCache;
use crate::store::service::{StoreService, UnifiedStoreGame};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};

/// Latest-package downloads share one provider lane. The per-AppID lock
/// coalesces duplicate UI/monitor requests, while the semaphore prevents
/// package ZIP requests for different games from becoming a rate-limit burst.
fn latest_package_gate() -> &'static Semaphore {
    static GATE: OnceLock<Semaphore> = OnceLock::new();
    GATE.get_or_init(|| Semaphore::new(1))
}

type LatestPackageLocks = AsyncMutex<HashMap<u32, Arc<AsyncMutex<()>>>>;

fn latest_package_locks() -> &'static LatestPackageLocks {
    static LOCKS: OnceLock<LatestPackageLocks> = OnceLock::new();
    LOCKS.get_or_init(|| AsyncMutex::new(HashMap::new()))
}

async fn latest_package_lock(app_id: u32) -> Arc<AsyncMutex<()>> {
    let mut locks = latest_package_locks().lock().await;
    locks
        .entry(app_id)
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

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

#[tauri::command]
pub async fn suggest_store_games(
    app: tauri::AppHandle,
    query: String,
) -> Result<Vec<StoreSuggestItem>, String> {
    let settings = load_settings(&app);
    let country = steam_country_code_for_currency(&settings.store_currency);
    let items = crate::steam::store::SteamStore::new()
        .suggest_for_country(query.trim(), country)
        .await?;
    Ok(items
        .into_iter()
        .map(|item| StoreSuggestItem {
            id: item.id,
            name: item.name,
            app_id: item.id.to_string(),
        })
        .collect())
}

#[tauri::command]
pub async fn search_store(
    app: tauri::AppHandle,
    query: String,
) -> Result<Vec<UnifiedStoreGame>, String> {
    let settings = load_settings(&app);
    let show_store_dlcs = settings.show_store_dlcs;
    let show_store_nsfw = settings.show_store_nsfw;
    let show_store_delisted = settings.show_store_delisted;
    let store_currency = normalize_store_currency(&settings.store_currency);
    let steam_country_code = steam_country_code_for_currency(&store_currency);
    let app_version = app.package_info().version.to_string();
    let info_cache_version = cache_version_with_currency(&app_version, &store_currency);
    let hubcap_client = (!settings.hubcap_api_key.trim().is_empty())
        .then(|| HubcapClient::new(settings.hubcap_api_key));
    let hubcap_checked = hubcap_client.is_some();

    crate::desk_log_info!("store", "Searching store for query='{}' (currency={}, dlcs={}, nsfw={}, delisted={}, hubcap_key_active={})",
        query, store_currency, show_store_dlcs, show_store_nsfw, show_store_delisted, hubcap_checked);

    let cache = StoreSearchCache::new(
        LocalAppPaths::data_root().join("cache"),
        app_version.clone(),
    );
    // The filter flags are part of the cache key: toggling any setting must
    // not replay 24h-stale results built under other flag values.
    let cache_key = build_store_cache_key(
        hubcap_client.is_some(),
        &store_currency,
        show_store_dlcs,
        show_store_nsfw,
        show_store_delisted,
        &query,
    );

    if let Some(results) = cache.get_fresh(&cache_key) {
        crate::desk_log_debug!("store", "Store search cache hit state=fresh query_len={} results={}", query.len(), results.len());
        GameInfoCache::new(
            LocalAppPaths::data_root().join("cache"),
            info_cache_version.clone(),
        )
        .merge_store_results_with_manifest_context(&results, hubcap_checked);
        return Ok(results);
    }

    match StoreService::new()
        .search_store(&query, hubcap_client, show_store_dlcs, show_store_nsfw, show_store_delisted, steam_country_code)
        .await
    {
        Ok(results) => {
            let cache_dir = LocalAppPaths::data_root().join("cache");
            SteamAppNameResolver::new(cache_dir.clone())
                .merge_names(results.iter().map(|game| (game.id, game.name.clone())));
            GameInfoCache::new(cache_dir, info_cache_version.clone())
                .merge_store_results_with_manifest_context(&results, hubcap_checked);
            match cache.put(&cache_key, results.clone()) {
                Ok(()) => crate::desk_log_debug!("store", "Store search cache write complete query_len={} results={}", query.len(), results.len()),
                Err(error) => crate::desk_log_warn!("store", "Store search cache write failed query_len={}: {}", query.len(), error),
            }
            crate::desk_log_info!("store", "Store search query_len={} completed results={}", query.len(), results.len());
            Ok(results)
        }
        Err(error) => {
            if let Some(results) = cache.get_any(&cache_key) {
                GameInfoCache::new(
                    LocalAppPaths::data_root().join("cache"),
                    info_cache_version.clone(),
                )
                .merge_store_results_with_manifest_context(&results, hubcap_checked);
                crate::desk_log_warn!("store", "Store search query='{}' network error ({}); served {} fallback cached result(s)", query, error, results.len());
                Ok(results)
            } else {
                crate::desk_log_error!("store", "Store search query='{}' failed: {}", query, error);
                Err(error)
            }
        }
    }
}

#[tauri::command]
pub async fn get_trending_store_games(
    app: tauri::AppHandle,
    start: usize,
    count: usize,
) -> Result<Vec<UnifiedStoreGame>, String> {
    let settings = load_settings(&app);
    if !settings.show_store_front_games {
        return Ok(Vec::new());
    }

    let show_store_dlcs = settings.show_store_dlcs;
    let show_store_nsfw = settings.show_store_nsfw;
    let show_store_delisted = settings.show_store_delisted;
    let store_front_filter = normalize_store_front_filter(&settings.store_front_filter);
    let store_currency = normalize_store_currency(&settings.store_currency);
    let steam_country_code = steam_country_code_for_currency(&store_currency);
    let app_version = app.package_info().version.to_string();
    let info_cache_version = cache_version_with_currency(&app_version, &store_currency);

    // The store front is Steam-only by design (Hubcap per-item status checks
    // tripped the IP rate limit during normal browsing), so the cache key has
    // no hubcap component and no Hubcap client is constructed here.
    let cache = StoreSearchCache::new(
        LocalAppPaths::data_root().join("cache"),
        app_version,
    );
    let cache_key = build_trending_cache_key(
        &store_currency,
        &store_front_filter,
        show_store_dlcs,
        show_store_nsfw,
        show_store_delisted,
        start,
        count,
    );

    if let Some(results) = cache.get_fresh_for(&cache_key, 24 * 60 * 60) {
        GameInfoCache::new(
            LocalAppPaths::data_root().join("cache"),
            info_cache_version.clone(),
        )
        .merge_store_results_with_manifest_context(&results, false);
        return Ok(results);
    }

    match StoreService::new()
        .trending_store(
            &store_front_filter,
            start,
            count,
            show_store_dlcs,
            show_store_nsfw,
            show_store_delisted,
            steam_country_code,
        )
        .await
    {
        Ok(results) => {
            let cache_dir = LocalAppPaths::data_root().join("cache");
            SteamAppNameResolver::new(cache_dir.clone())
                .merge_names(results.iter().map(|game| (game.id, game.name.clone())));
            GameInfoCache::new(cache_dir, info_cache_version)
                .merge_store_results_with_manifest_context(&results, false);
            let _ = cache.put(&cache_key, results.clone());
            Ok(results)
        }
        Err(error) => {
            if let Some(results) = cache.get_any(&cache_key) {
                Ok(results)
            } else {
                Err(error)
            }
        }
    }
}

#[tauri::command]
pub fn get_cached_store_search(
    app: tauri::AppHandle,
    query: String,
) -> Result<CachedStoreSearchResponse, String> {
    if query.trim().is_empty() {
        return Ok(CachedStoreSearchResponse {
            results: Vec::new(),
            cache_state: "miss".to_string(),
        });
    }

    let settings = load_settings(&app);
    let store_currency = normalize_store_currency(&settings.store_currency);
    let hubcap_enabled = !settings.hubcap_api_key.trim().is_empty();
    let cache = StoreSearchCache::new(
        LocalAppPaths::data_root().join("cache"),
        app.package_info().version.to_string(),
    );
    let cache_key = build_store_cache_key(
        hubcap_enabled,
        &store_currency,
        settings.show_store_dlcs,
        settings.show_store_nsfw,
        settings.show_store_delisted,
        &query,
    );

    if let Some(results) = cache.get_fresh(&cache_key) {
        return Ok(CachedStoreSearchResponse {
            results,
            cache_state: "fresh".to_string(),
        });
    }

    if let Some(results) = cache.get_stale(&cache_key) {
        return Ok(CachedStoreSearchResponse {
            results,
            cache_state: "stale".to_string(),
        });
    }

    Ok(CachedStoreSearchResponse {
        results: Vec::new(),
        cache_state: "miss".to_string(),
    })
}

#[tauri::command]
pub async fn check_denuvo_bulk(
    app: tauri::AppHandle,
    app_ids: Vec<u32>,
) -> Result<HashMap<u32, bool>, String> {
    let cache_dir = LocalAppPaths::data_root().join("cache");
    let app_version = app.package_info().version.to_string();
    let settings = load_settings(&app);
    let info_cache_version = cache_version_with_currency(&app_version, &settings.store_currency);
    let results = DrmDetector::new(cache_dir.clone(), app_version)
        .detect_many(app_ids)
        .await?;
    GameInfoCache::new(cache_dir, info_cache_version).merge_denuvo_flags(&results);
    Ok(results)
}

#[tauri::command]
pub async fn trigger_hubcap_download(
    app: tauri::AppHandle,
    app_id: u32,
    api_key: String,
) -> Result<String, String> {
    // Il percorso Steam viene dalle impostazioni (unico punto di verità):
    // `validate_download_inputs` continua a validarlo e a loggare l'errore.
    let steam_path = command_steam_path(&app)?;
    if let Err(e) = validate_download_inputs(&api_key, &steam_path, "call Hubcap Manifest") {
        crate::desk_log_error!("store", "Download failed for {}: {}", crate::core::logger::format_appid(app_id), e);
        return Err(e);
    }

    let app_lock = latest_package_lock(app_id).await;
    let _app_guard = app_lock.lock().await;
    let _provider_guard = latest_package_gate()
        .acquire()
        .await
        .map_err(|_| "Hubcap package scheduler is unavailable".to_string())?;

    crate::desk_log_info!("store", "Triggering download for {} (source: {})",
        crate::core::logger::format_appid(app_id), if api_key == "oureveryday_public" { "oureveryday" } else { "hubcap" });

    // Provider acquisition and Steam publication are intentionally separate:
    // every latest-download source converges on the same verified installer.
    let source = if api_key == "oureveryday_public" {
        "MOED"
    } else {
        "Hubcap"
    };
    let res = async {
        let mut package = if api_key == "oureveryday_public" {
            crate::providers::oureveryday::OureverydayClient::new()
                .download_lua_package(app_id)
                .await?
        } else {
            let hubcap = HubcapClient::new(api_key.clone());
            // Validation is deduplicated session-wide by the shared client
            // cache, so repeated downloads within the TTL cost no round-trip.
            if !hubcap.validate_api_key().await? {
                return Err("Hubcap API key is not valid or is not allowed to make requests.".to_string());
            }
            hubcap.download_lua_package(app_id).await?
        };
        if api_key != "oureveryday_public" {
            let generated = prepare_hubcap_manifest_files(
                app_id,
                &steam_path,
                &package.lua_content,
                &package.manifest_files,
                &api_key,
            )
            .await?;
            package.manifest_files.extend(generated);
        }
        install_standard_package(&app, app_id, &steam_path, package, source).await
    }
    .await;

    match &res {
        Ok(msg) => crate::desk_log_info!("store", "Successfully completed download for {}: {}", crate::core::logger::format_appid(app_id), msg),
        Err(e) => crate::desk_log_error!("store", "Download failed for {} (source: {}): {}", crate::core::logger::format_appid(app_id), if api_key == "oureveryday_public" { "oureveryday" } else { "hubcap" }, e),
    }
    res
}

#[tauri::command]
pub async fn prepare_specific_version_download(
    app: tauri::AppHandle,
    app_id: u32,
    api_key: String,
) -> Result<Vec<LuaManifestRow>, String> {
    let steam_path = command_steam_path(&app)?;
    if let Err(e) = validate_download_inputs(&api_key, &steam_path, "download the Lua file") {
        crate::desk_log_error!("store", "Specific version download failed for {}: {}", crate::core::logger::format_appid(app_id), e);
        return Err(e);
    }

    crate::desk_log_info!("store", "Preparing specific version download for {} (source key: {})", crate::core::logger::format_appid(app_id), if api_key == "oureveryday_public" { "oureveryday" } else { "hubcap" });

    let res = async {
        let package = if api_key == "oureveryday_public" {
            let oe_client = crate::providers::oureveryday::OureverydayClient::new();
            oe_client.download_lua_package(app_id).await?
        } else {
            let hubcap = HubcapClient::new(api_key.clone());
            if !hubcap.validate_api_key().await? {
                return Err("Hubcap API key is not valid or is not allowed to make requests.".to_string());
            }
            hubcap.download_lua_package(app_id).await?
        };
        let lua_content = package.lua_content;
        let manifest_rows = LuaManifestPins::rows_from_content(&lua_content);

        if manifest_rows.is_empty() {
            return Err("The downloaded Lua does not contain any setManifestid entries, so it was not installed. Try another source or verify the provider returned the full Lua with manifests.".to_string());
        }

        let steam = SteamCompat::new(steam_path.clone());
        // Local-first (B1): bundled files install with the package itself, so
        // only the remaining pins are resolved — backup/secondary-cache hits
        // are restored into depotcache first, and only genuinely absent pins
        // are generated (requiring an authenticated Hubcap key).
        let bundled = bundled_manifest_names(&package.manifest_files);
        let pins_to_resolve: Vec<DepotManifestPin> = enabled_pins_from_content(&lua_content)
            .into_iter()
            .filter(|pin| !bundled.contains(&resolver::manifest_file_name(pin)))
            .collect();
        let generation = if api_key == "oureveryday_public" {
            resolver::Generation::LocalOnly
        } else {
            resolver::Generation::PreValidatedKey(api_key.clone())
        };
        let resolution = resolver::resolve(resolver::ManifestRequest {
            steam_path: steam_path.clone(),
            app_id,
            pins: pins_to_resolve,
            generation,
        })
        .await?;
        if !resolution.is_complete() {
            return Err("This specific version includes manifests not present locally. Configure a valid Hubcap API key; the obsolete request-code path is not used.".to_string());
        }
        let generated = resolution.generated;
        let mut backup_manifests = package.manifest_files.clone();
        backup_manifests.extend(generated);
        steam.install_lua_and_manifest_files(app_id, &lua_content, &backup_manifests)?;
        verify_referenced_manifests(app_id, &steam_path, &lua_content)?;
        apply_update_policy_and_backup(&app, app_id, &steam_path, &lua_content, &backup_manifests)?;

        let installed_rows = LuaManifestPins::new(steam_path, app_id).rows_from_file()?;
        if installed_rows.len() != manifest_rows.len() {
            return Err(format!(
                "Lua install verification failed: downloaded file had {} setManifestid entries, installed file has {}.",
                manifest_rows.len(), installed_rows.len()
            ));
        }

        crate::core::library_events::notify_lua_changed(
            &app,
            crate::core::library_events::LibraryChangeOrigin::Store,
            [app_id],
        );
        Ok(installed_rows)
    }.await;

    match &res {
        Ok(rows) => crate::desk_log_info!("store", "Successfully prepared specific version download for {}: {} row(s) installed", crate::core::logger::format_appid(app_id), rows.len()),
        Err(e) => crate::desk_log_error!("store", "Specific version download failed for {}: {}", crate::core::logger::format_appid(app_id), e),
    }
    res
}

#[tauri::command]
pub async fn trigger_ryuu_download(
    app: tauri::AppHandle,
    app_id: u32,
    api_key: String,
) -> Result<String, String> {
    let steam_path = command_steam_path(&app)?;
    if let Err(e) = validate_download_inputs(&api_key, &steam_path, "call Ryuu") {
        crate::desk_log_error!("store", "Ryuu download failed for {}: {}", crate::core::logger::format_appid(app_id), e);
        return Err(e);
    }

    crate::desk_log_info!("store", "Triggering Ryuu download for {}", crate::core::logger::format_appid(app_id));

    let res = async {
        let package = RyuuClient::new(api_key).download_lua_package(app_id).await?;
        install_standard_package(&app, app_id, &steam_path, package, "Ryuu").await
    }
    .await;

    match &res {
        Ok(msg) => crate::desk_log_info!("store", "Successfully completed Ryuu download for {}: {}", crate::core::logger::format_appid(app_id), msg),
        Err(e) => crate::desk_log_error!("store", "Ryuu download failed for {}: {}", crate::core::logger::format_appid(app_id), e),
    }
    res
}

#[tauri::command]
pub async fn prepare_ryuu_specific_version_download(
    app: tauri::AppHandle,
    app_id: u32,
    api_key: String,
) -> Result<Vec<LuaManifestRow>, String> {
    let steam_path = command_steam_path(&app)?;
    validate_download_inputs(&api_key, &steam_path, "download the Lua file from Ryuu")?;
    crate::desk_log_info!("store", "Preparing Ryuu specific version download for {}", crate::core::logger::format_appid(app_id));

    let package = RyuuClient::new(api_key).download_lua_package(app_id).await?;
    let installed_rows =
        install_specific_package(&app, app_id, &steam_path, package, "Ryuu").await?;
    crate::desk_log_info!("store", "Successfully prepared Ryuu specific version download for {}: {} row(s) installed", crate::core::logger::format_appid(app_id), installed_rows.len());
    Ok(installed_rows)
}

#[tauri::command]
pub async fn trigger_luatools_download(
    app: tauri::AppHandle,
    app_id: u32,
    game_name: Option<String>,
) -> Result<String, String> {
    let steam_path = command_steam_path(&app)?;
    validate_steam_download_path(&steam_path)?;
    crate::desk_log_info!(
        "store",
        "Triggering authenticated LuaTools download for {}",
        crate::core::logger::format_appid(app_id)
    );
    let package = download_complete_luatools_package(&app, app_id, &steam_path, game_name.as_deref()).await?;
    install_standard_package(&app, app_id, &steam_path, package, "LuaTools").await
}

#[tauri::command]
pub async fn prepare_luatools_specific_version_download(
    app: tauri::AppHandle,
    app_id: u32,
    game_name: Option<String>,
) -> Result<Vec<LuaManifestRow>, String> {
    let steam_path = command_steam_path(&app)?;
    validate_steam_download_path(&steam_path)?;
    crate::desk_log_info!(
        "store",
        "Preparing LuaTools specific-version download for {}",
        crate::core::logger::format_appid(app_id)
    );
    let package = download_complete_luatools_package(&app, app_id, &steam_path, game_name.as_deref()).await?;
    install_specific_package(&app, app_id, &steam_path, package, "LuaTools").await
}

/// How many LuaTools sources one download may try. Each attempt is a package
/// download that counts against the account's daily allowance, so the second
/// source is only tried when the first one's package could not be completed
/// (a bare Lua whose manifests neither LuaTools nor Hubcap could provide) —
/// the alternative at that point is a failed install, not a saved download.
const LUATOOLS_MAX_SOURCE_ATTEMPTS: usize = 2;

/// Downloads a LuaTools package that is guaranteed complete (Lua + every
/// enabled manifest) or fails before anything reaches Steam: best source
/// first, completion through [`complete_luatools_package`], one fallback
/// source when the first package cannot be completed.
async fn download_complete_luatools_package(
    app: &tauri::AppHandle,
    app_id: u32,
    steam_path: &str,
    game_name: Option<&str>,
) -> Result<ManifestPackage, String> {
    let client = LuaToolsClient::new();
    let sources = client.available_sources(app_id).await?;
    let attempts = sources.len().min(LUATOOLS_MAX_SOURCE_ATTEMPTS);
    let mut previous_failure: Option<(String, String)> = None;
    for (index, source) in sources.iter().take(attempts).enumerate() {
        let mut package = match client.download_lua_package_from(app_id, source, game_name).await {
            Ok(package) => package,
            Err(error) => {
                return Err(match previous_failure {
                    Some((first_source, first_error)) => format!(
                        "{error} (fallback after source {first_source} could not be completed: {first_error})"
                    ),
                    None => error,
                });
            }
        };
        match complete_luatools_package(app, app_id, steam_path, &client, &mut package).await {
            Ok(()) => return Ok(package),
            Err(error) => {
                let next = sources.get(index + 1).filter(|_| index + 1 < attempts);
                match next {
                    Some(next_source) => {
                        crate::desk_log_warn!(
                            "store",
                            "LuaTools source '{}' package for {} could not be completed ({}); trying source '{}'",
                            source,
                            crate::core::logger::format_appid(app_id),
                            error,
                            next_source
                        );
                        previous_failure = Some((source.clone(), error));
                    }
                    None => {
                        return Err(match previous_failure {
                            Some((first_source, first_error)) => format!(
                                "{error} (source {first_source} was tried first: {first_error})"
                            ),
                            None => error,
                        });
                    }
                }
            }
        }
    }
    Err(format!("LuaTools has no available source for App ID {app_id}"))
}

/// Completes a LuaTools package before it reaches the shared installer.
///
/// LuaTools sources come in two families: the ones that archive manifests
/// answer `/api/manifest/download` with `<appid>.zip` (Lua + `.manifest`
/// files), the ones that only mirror the entitlement file (e.g. Luie) answer
/// with a bare pinned Lua whose `setManifestid` rows reference manifests that
/// are nowhere on disk yet. The manifests-before-Lua contract does not bend
/// for them: the completion gate in the installer would (correctly) refuse
/// the package with "N referenced manifest(s) are missing". So, local-first:
///
/// 1. pins already bundled in the package or already present in depotcache /
///    Steam's secondary cache / the AetherData backup are left alone;
/// 2. every other enabled pin is fetched through LuaTools' own per-depot
///    endpoint (same signed-in session, verified depot/GID, no daily-cap
///    cost);
/// 3. only what LuaTools could not serve is generated through Hubcap, and
///    only when the user configured a key — never a silent quota spend.
///
/// Anything still missing afterwards is reported here with the provider's
/// reasons, before a single byte is written to Steam.
async fn complete_luatools_package(
    app: &tauri::AppHandle,
    app_id: u32,
    steam_path: &str,
    client: &LuaToolsClient,
    package: &mut ManifestPackage,
) -> Result<(), String> {
    let bundled = bundled_manifest_names(&package.manifest_files);
    let pins: Vec<DepotManifestPin> = enabled_pins_from_content(&package.lua_content)
        .into_iter()
        .filter(|pin| !bundled.contains(&resolver::manifest_file_name(pin)))
        .collect();
    if pins.is_empty() {
        crate::desk_log_info!(
            "store",
            "LuaTools package for {} is self-contained: {} manifest(s) bundled, nothing to complete",
            crate::core::logger::format_appid(app_id),
            package.manifest_files.len()
        );
        return Ok(());
    }
    let completeness = resolver::verify_available(steam_path, app_id, &pins);
    if completeness.missing.is_empty() {
        crate::desk_log_info!(
            "store",
            "LuaTools package for {}: {} pin(s) not bundled but already available locally",
            crate::core::logger::format_appid(app_id),
            completeness.verified
        );
        return Ok(());
    }
    crate::desk_log_info!(
        "store",
        "LuaTools package for {} needs {} manifest(s) (bundled={}, local={}): completing through LuaTools",
        crate::core::logger::format_appid(app_id),
        completeness.missing.len(),
        package.manifest_files.len(),
        completeness.verified
    );

    let completion = client
        .complete_manifests(app_id, &completeness.missing)
        .await;
    let fetched_count = completion.fetched.len();
    package.manifest_files.extend(completion.fetched);
    if completion.failed.is_empty() {
        crate::desk_log_info!(
            "store",
            "LuaTools completion for {}: {} manifest(s) fetched, package complete",
            crate::core::logger::format_appid(app_id),
            fetched_count
        );
        return Ok(());
    }

    // Hubcap fallback — exact generation of the leftovers, only with a key.
    let still_missing: Vec<DepotManifestPin> = completion
        .failed
        .iter()
        .map(|(pin, _)| pin.clone())
        .collect();
    let hubcap_key = load_settings(app).hubcap_api_key.trim().to_string();
    let mut unresolved = still_missing.clone();
    if !hubcap_key.is_empty() && hubcap_key != "oureveryday_public" {
        crate::desk_log_info!(
            "store",
            "LuaTools completion for {}: {} manifest(s) not served by LuaTools, trying Hubcap generation",
            crate::core::logger::format_appid(app_id),
            still_missing.len()
        );
        match resolver::resolve(resolver::ManifestRequest {
            steam_path: steam_path.to_string(),
            app_id,
            pins: still_missing,
            generation: resolver::Generation::SettingsKey(hubcap_key.clone()),
        })
        .await
        {
            Ok(resolution) => {
                package.manifest_files.extend(resolution.generated);
                unresolved = resolution.missing;
            }
            Err(error) => crate::desk_log_warn!(
                "store",
                "Hubcap fallback for LuaTools package {} failed: {}",
                crate::core::logger::format_appid(app_id),
                error
            ),
        }
    }

    if unresolved.is_empty() {
        crate::desk_log_info!(
            "store",
            "LuaTools completion for {}: {} fetched from LuaTools, rest generated through Hubcap, package complete",
            crate::core::logger::format_appid(app_id),
            fetched_count
        );
        return Ok(());
    }

    let reasons: Vec<String> = completion
        .failed
        .iter()
        .filter(|(pin, _)| unresolved.contains(pin))
        .map(|(pin, reason)| format!("{}_{} ({})", pin.depot_id, pin.manifest_id, reason))
        .collect();
    crate::desk_log_error!(
        "store",
        "LuaTools completion for {} incomplete: {} manifest(s) unavailable: {}",
        crate::core::logger::format_appid(app_id),
        unresolved.len(),
        reasons.join("; ")
    );
    let hint = if hubcap_key.is_empty() {
        " Configure a Hubcap API key in Settings to generate the missing manifests, or retry later."
    } else {
        " Retry later or try another source."
    };
    Err(format!(
        "LuaTools could not provide {} of the {} manifest(s) this Lua references: {}.{}",
        unresolved.len(),
        pins.len(),
        reasons.join("; "),
        hint
    ))
}

// ============================================================================
// Local-first manifest completion
//
// The heavy lifting (restore from backup/secondary cache, generation,
// completeness gate) lives in `manifest::resolver`, the single path shared
// with versioning, library edits, and the explicit repair command. The
// helpers below only adapt package-shaped inputs to that resolver.
// ============================================================================

/// Pins whose Lua rows are enabled: the manifests Steam will actually request.
fn enabled_pins_from_content(lua_content: &str) -> Vec<DepotManifestPin> {
    pins_from_rows(
        LuaManifestPins::rows_from_content(lua_content)
            .into_iter()
            .filter(|row| row.enabled),
    )
}

/// Non-empty manifest files carried by a downloaded package, by file name.
fn bundled_manifest_names(package_files: &[ManifestPackageFile]) -> HashSet<String> {
    package_files
        .iter()
        .filter_map(|manifest| {
            let name = std::path::Path::new(&manifest.file_name)
                .file_name()
                .and_then(|value| value.to_str())?;
            (!manifest.bytes.is_empty()).then_some(name.to_string())
        })
        .collect()
}

/// Completes a Hubcap package with exact manifest generation for rows that
/// were not included in the ZIP and cannot be satisfied from local caches.
async fn prepare_hubcap_manifest_files(
    app_id: u32,
    steam_path: &str,
    lua_content: &str,
    package_files: &[ManifestPackageFile],
    api_key: &str,
) -> Result<Vec<ManifestPackageFile>, String> {
    // Local-first (B1): bundled files install with the package itself, so
    // only the remaining pins are resolved — anything already on disk is
    // restored (never regenerated), and only genuinely absent pins are
    // generated through the already-validated key.
    let bundled = bundled_manifest_names(package_files);
    let pins: Vec<DepotManifestPin> = enabled_pins_from_content(lua_content)
        .into_iter()
        .filter(|pin| !bundled.contains(&resolver::manifest_file_name(pin)))
        .collect();
    let generation = if api_key.trim().is_empty() || api_key == "oureveryday_public" {
        resolver::Generation::LocalOnly
    } else {
        resolver::Generation::PreValidatedKey(api_key.to_string())
    };
    let resolution = resolver::resolve(resolver::ManifestRequest {
        steam_path: steam_path.to_string(),
        app_id,
        pins,
        generation,
    })
    .await?;
    if resolution.is_complete() {
        Ok(resolution.generated)
    } else {
        Err("This package needs manifest files that are not stored locally. Configure a valid Hubcap API key; the obsolete request-code path is not used.".to_string())
    }
}

fn verify_referenced_manifests(
    app_id: u32,
    steam_path: &str,
    lua_content: &str,
) -> Result<(), String> {
    // Shared completeness gate (`manifest::resolver`): the exact same
    // "present" definition used by local-first resolution — depotcache,
    // Steam's secondary cache, or the per-game AetherData backup.
    let pins = enabled_pins_from_content(lua_content);
    let completeness = resolver::verify_available(steam_path, app_id, &pins);
    if !completeness.missing.is_empty() {
        let missing = completeness
            .missing
            .iter()
            .map(|pin| format!("{}_{}", pin.depot_id, pin.manifest_id))
            .collect::<Vec<_>>()
            .join(", ");
        crate::desk_log_error!(
            "store",
            "Manifest completion gate failed app_id={} missing={}",
            app_id,
            completeness.missing.len()
        );
        return Err(format!(
            "Package was not completed: {} referenced manifest(s) are missing or empty after install: {}",
            completeness.missing.len(),
            missing
        ));
    }
    crate::desk_log_info!(
        "store",
        "Manifest completion gate passed app_id={} verified={}",
        app_id,
        completeness.verified
    );
    Ok(())
}

async fn install_standard_package(
    app: &tauri::AppHandle,
    app_id: u32,
    steam_path: &str,
    package: ManifestPackage,
    source: &str,
) -> Result<String, String> {
    let steam = SteamCompat::new(steam_path.to_string());
    // Deterministic auto-download (P1): failed Steam downloads leave dirty
    // partial state under steamapps/downloading/<appid>. The Lua commit below
    // is the DLL hot-reload trigger that makes Steam reconcile at once, and
    // it would RESUME that dirty state (stalled speed, phantom sizes,
    // connection errors) instead of starting clean. Remove it first.
    steam.clear_residual_download_state(app_id);
    steam.install_lua_and_manifest_files(app_id, &package.lua_content, &package.manifest_files)?;
    // Local-first completeness (B1): publish anything restorable from disk
    // before the gate runs. No-op when package planning already restored
    // everything (Hubcap path); this also covers provider packages that are
    // never completed (Ryuu / LuaTools / MOED).
    resolver::resolve(resolver::ManifestRequest {
        steam_path: steam_path.to_string(),
        app_id,
        pins: enabled_pins_from_content(&package.lua_content),
        generation: resolver::Generation::LocalOnly,
    })
    .await?;
    verify_referenced_manifests(app_id, steam_path, &package.lua_content)?;
    apply_update_policy_and_backup(
        app,
        app_id,
        steam_path,
        &package.lua_content,
        &package.manifest_files,
    )?;
    // Pre-stage what Hubcap currently packages beyond the pins this Lua
    // carries (P1, Hubcap-only): free contents diff, generation only for
    // genuinely missing manifests, commented-pin realignment when the update
    // policy left the game in updates-ON mode. Soft-fail by design — the
    // package is committed already and the monitor's pin_refresh lane retries
    // periodically, so a provider hiccup must not fail a successful install.
    match crate::commands::manifests::refresh_game_pins_from_hubcap(app.clone(), app_id).await {
        Ok(report) if report.staged > 0 || report.realigned > 0 => crate::desk_log_info!(
            "store",
            "Post-install pin refresh app_id={} staged={} realigned={}",
            app_id,
            report.staged,
            report.realigned
        ),
        Ok(_) => {}
        Err(error) => crate::desk_log_warn!(
            "store",
            "Post-install pin refresh not applied app_id={}: {}",
            app_id,
            error
        ),
    }
    crate::core::library_events::notify_lua_changed(
        app,
        crate::core::library_events::LibraryChangeOrigin::Store,
        [app_id],
    );
    Ok(format!(
        "Successfully completed {} download for App ID {}. Lua installed, {} manifest file(s) preloaded into Steam depotcache.",
        source,
        app_id,
        package.manifest_files.len()
    ))
}

async fn install_specific_package(
    app: &tauri::AppHandle,
    app_id: u32,
    steam_path: &str,
    package: ManifestPackage,
    source: &str,
) -> Result<Vec<LuaManifestRow>, String> {
    let manifest_rows = LuaManifestPins::rows_from_content(&package.lua_content);
    if manifest_rows.is_empty() {
        return Err(format!(
            "The downloaded Lua from {} does not contain any setManifestid entries, so it was not installed.",
            source
        ));
    }

    let steam = SteamCompat::new(steam_path.to_string());
    // Same residual-state cleanup as the latest-version installer: a version
    // switch must never resume dirty chunks of a previous failed download.
    steam.clear_residual_download_state(app_id);
    steam.install_lua_and_manifest_files(app_id, &package.lua_content, &package.manifest_files)?;
    // Local-first completeness (B1): same restore-before-gate contract as the
    // latest-version installer (covers the Ryuu / LuaTools packages, which
    // are never completed with generated manifests).
    resolver::resolve(resolver::ManifestRequest {
        steam_path: steam_path.to_string(),
        app_id,
        pins: enabled_pins_from_content(&package.lua_content),
        generation: resolver::Generation::LocalOnly,
    })
    .await?;
    verify_referenced_manifests(app_id, steam_path, &package.lua_content)?;
    // Same default update policy as latest-version installs (B3): when the
    // user opted into updates, pins start commented and stay editable in the
    // specific-version modal that opens right after this install.
    apply_update_policy_and_backup(
        app,
        app_id,
        steam_path,
        &package.lua_content,
        &package.manifest_files,
    )?;

    let installed_rows = LuaManifestPins::new(steam_path.to_string(), app_id).rows_from_file()?;
    if installed_rows.len() != manifest_rows.len() {
        return Err(format!(
            "Lua install verification failed: downloaded file had {} setManifestid entries, installed file has {}.",
            manifest_rows.len(),
            installed_rows.len()
        ));
    }
    crate::core::library_events::notify_lua_changed(
        app,
        crate::core::library_events::LibraryChangeOrigin::Store,
        [app_id],
    );
    Ok(installed_rows)
}

fn validate_steam_download_path(steam_path: &str) -> Result<(), String> {
    // Full validation (exists + is a dir + contains steam.exe): a typo'd path
    // must fail here, before any provider download or Steam-side write.
    crate::util::validation::validate_steam_path(steam_path)
}

fn validate_download_inputs(
    api_key: &str,
    steam_path: &str,
    api_action: &str,
) -> Result<(), String> {
    if api_key.trim().is_empty() {
        let err = format!("API Key is required to {}", api_action);
        crate::desk_log_error!("store", "Download validation failed: {}", err);
        return Err(err);
    }
    if let Err(err) = crate::util::validation::validate_steam_path(steam_path) {
        crate::desk_log_error!("store", "Download validation failed: {}", err);
        return Err(err);
    }
    Ok(())
}

/// Shared tail of every store package install: applies the user's default
/// update policy, then archives the Lua exactly as it now exists on disk
/// (the policy may have commented pins — the backup must mirror what Steam
/// actually reads, never a state the disk no longer has) together with the
/// package manifests.
fn apply_update_policy_and_backup(
    app: &tauri::AppHandle,
    app_id: u32,
    steam_path: &str,
    lua_fallback: &str,
    manifests: &[ManifestPackageFile],
) -> Result<(), String> {
    apply_default_update_policy(app, app_id, steam_path)?;
    let installed_lua = SteamCompat::new(steam_path.to_string())
        .read_lua_config(app_id)
        .unwrap_or_else(|_| lua_fallback.to_string());
    GameBackup::for_app(app_id)?
        .backup_lua_artifacts(app_id, &installed_lua, manifests)?;
    Ok(())
}

fn apply_default_update_policy(
    app: &tauri::AppHandle,
    app_id: u32,
    steam_path: &str,
) -> Result<(), String> {
    let settings = load_settings(app);
    crate::desk_log_debug!(
        "store",
        "Update policy after package install app_id={} enabled={}",
        app_id,
        settings.download_games_with_updates_on
    );
    if settings.download_games_with_updates_on {
        LuaManifestPins::new(steam_path.to_string(), app_id)
            .set_updates_enabled(true)
            .map(|_| ())?;
    }
    Ok(())
}

fn build_store_cache_key(
    hubcap_enabled: bool,
    store_currency: &str,
    show_store_dlcs: bool,
    show_store_nsfw: bool,
    show_store_delisted: bool,
    query: &str,
) -> String {
    format!(
        "{}|currency={}|dlcs={}|nsfw={}|delisted={} {}",
        if hubcap_enabled { "hubcap" } else { "steam" },
        store_currency,
        show_store_dlcs,
        show_store_nsfw,
        show_store_delisted,
        query
    )
}

fn build_trending_cache_key(
    store_currency: &str,
    store_front_filter: &str,
    show_store_dlcs: bool,
    show_store_nsfw: bool,
    show_store_delisted: bool,
    start: usize,
    count: usize,
) -> String {
    format!(
        "storefront={}|steam|currency={}|dlcs={}|nsfw={}|delisted={}|start={}|count={}",
        store_front_filter,
        store_currency,
        show_store_dlcs,
        show_store_nsfw,
        show_store_delisted,
        start,
        count,
    )
}
