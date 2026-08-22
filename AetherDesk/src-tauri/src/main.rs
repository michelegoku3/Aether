#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod core;
mod crack;
mod external_tools;
mod game_info;
mod local;
mod manifest;
mod online;
mod providers;
mod steam;
mod steamless;
mod store;
mod updater;
mod util;
mod versioning;

#[cfg(test)]
mod tests;

fn main() {
    // Portable helper entry points (no UI window). Must run before Tauri setup.
    //   --apply-update <staging> <install_root>
    //   --uninstall-desk <install_root> (--delete-user-data | --keep-user-data)
    if let Some(code) = try_run_apply_update() {
        std::process::exit(code);
    }
    if let Some(code) = try_run_uninstall_desk() {
        std::process::exit(code);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Initialize the session logger (rotates desk.log -> desk.log.last on startup).
            crate::core::logger::init(&app.handle());
            // UCOnline2 appends to %TEMP%\uc_online2.log while games run; remove it
            // at startup so the log shown/exported by AetherDesk is always clean
            // (UCO2 recreates it automatically on the next game launch).
            crate::commands::logs::clear_uco2_log_file();
            // Clear any leftover artifacts from an interrupted portable self-update.
            crate::updater::desk::cleanup_stale_artifacts();
            // All startup migrations live in one place: legacy settings move,
            // obsolete data-folder cleanup, and the lua_backups → backup data
            // layout migration. Each step is idempotent and failure-tolerant.
            crate::core::migration::run_startup_migrations(&app.handle());
            // Lua backup sync (background, non-blocking): mirror every .lua in
            // stplug-in into backup/<app_id>/lua — creates missing backups and
            // archives+updates changed ones (history/ keeps old versions).
            {
                let app_handle = app.handle().clone();
                let steam_path =
                    crate::core::settings::SettingsManager::new(&app_handle).load().steam_path;
                tauri::async_runtime::spawn(async move {
                    let report = tauri::async_runtime::spawn_blocking(move || {
                        crate::core::backup::sync_lua_backups_from_stplug_in(
                            std::path::Path::new(&steam_path),
                        )
                    })
                    .await;
                    if let Ok(report) = report {
                        if report.created > 0 || report.updated > 0 {
                            crate::desk_log_info!(
                                "backup",
                                "Startup Lua backup sync: {} scanned, {} created, {} updated, {} unchanged, {} skipped",
                                report.scanned, report.created, report.updated,
                                report.unchanged, report.skipped
                            );
                        } else {
                            crate::desk_log_debug!(
                                "backup",
                                "Startup Lua backup sync: everything up to date ({} scanned, {} unchanged, {} skipped)",
                                report.scanned, report.unchanged, report.skipped
                            );
                        }
                    }
                });
            }
            // Background worker that retries ACF build edits queued by the
            // version pipeline (ACF missing until download, or held by Steam).
            crate::versioning::queue::spawn_retry_worker(app.handle().clone());
            if let Err(e) = crate::core::custom_css::apply_window_icon(&app.handle()) {
                eprintln!("[AetherDesk] window icon apply failed: {e}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::settings::validate_hubcap_key,
            commands::settings::get_hubcap_usage,
            commands::settings::get_luatools_auth_status,
            commands::settings::sign_in_luatools,
            commands::settings::cancel_luatools_sign_in,
            commands::settings::sign_in_luatools_with_code,
            commands::settings::sign_out_luatools,
            commands::settings::clear_app_caches,
            commands::settings::open_webview_devtools,
            commands::custom_css::get_custom_css,
            commands::custom_css::get_custom_css_path,
            commands::custom_css::get_personal_wallpaper_path,
            commands::custom_css::get_personal_wallpaper_data_uri,
            commands::custom_css::ensure_custom_css,
            commands::fs::reveal_in_file_manager,
            commands::custom_css::get_appearance_assets,
            commands::custom_css::pick_theme_file,
            commands::custom_css::pick_wallpaper_file,
            commands::custom_css::pick_icon_file,
            commands::custom_css::apply_window_icon,
            commands::crack::pick_crack_files,
            commands::crack::apply_crack,
            commands::crack::has_saved_crack,
            commands::crack::reapply_saved_crack,
            commands::crack::remove_applied_crack,
            commands::local::pick_local_files,
            commands::local::install_local_game,
            commands::local::list_lua_history,
            commands::antivirus::get_antivirus_exclusion_done,
            commands::antivirus::acknowledge_antivirus_exclusion,
            commands::antivirus::apply_antivirus_exclusion,
            commands::antivirus::open_windows_security,
            commands::antivirus::open_app_folder,
            commands::store::suggest_store_games,
            commands::store::search_store,
            commands::store::get_cached_store_search,
            commands::store::get_trending_store_games,
            commands::store::check_denuvo_bulk,
            commands::store::trigger_hubcap_download,
            commands::store::trigger_luatools_download,
            commands::store::trigger_ryuu_download,
            commands::store::prepare_specific_version_download,
            commands::store::prepare_luatools_specific_version_download,
            commands::store::prepare_ryuu_specific_version_download,
            commands::library::get_installed_library_games,
            commands::library::warm_library_game_cache,
            commands::library::open_steamdb_depots,
            commands::library::open_steamdb_patchnotes,
            commands::home_links::open_home_resource,
            commands::game_info::get_game_info,
            commands::steamless::pick_and_run_steamless,
            commands::online::get_online_status,
            commands::online::get_online_preferences,
            commands::online::clear_online_preferences,
            commands::online::is_uco2_active,
            commands::online::plan_online,
            commands::online::enable_online,
            commands::online::disable_online,
            commands::library::get_installed_lua_manifest_rows,
            commands::library::get_lua_game_update_state,
            commands::library::set_lua_game_updates_enabled,
            commands::library::remove_lua_game_from_library,
            commands::library::apply_specific_version_edits,
            commands::steam::restart_steam,
            commands::steam::is_dll_installed,
            commands::steam::is_steam_blocked,
            commands::steam::block_steam_updates,
            commands::steam::unblock_steam_updates,
            commands::steam::get_aether_onlinefix,
            commands::steam::set_aether_onlinefix,            commands::aether_dll::get_installed_dll_version,
            commands::aether_dll::check_aether_dll_update,
            commands::aether_dll::install_aether_dll,
            commands::aether_dll::uninstall_aether_dll,
            commands::aether_dll::reset_aether_steam_path,
            commands::aether_dll::probe_aether_steam_residuals,
            commands::aether_desk::get_desk_version,
            commands::aether_desk::check_aether_desk_update,
            commands::aether_desk::install_aether_desk_update,
            commands::aether_desk::restore_stable_desk,
            commands::aether_desk::uninstall_aether_desk,
            commands::logs::get_recent_log_lines,
            commands::logs::clear_session_log,
            commands::logs::export_logs_bundle,
            commands::logs::export_log_source,
            commands::logs::set_session_log_level,
            commands::versioning::get_game_builds,
            commands::versioning::get_saved_builds,
            commands::versioning::save_build,
            commands::versioning::remove_saved_build,
            commands::versioning::apply_game_version,
            commands::versioning::get_pending_version_edits,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Handles the `--apply-update <staging> <install_root>` invocation used by the
/// portable self-updater. Returns `Some(exit_code)` when the flag is present
/// (the process should terminate with that code), `None` otherwise.
fn try_run_apply_update() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let position = args.iter().position(|arg| arg == "--apply-update")?;
    let staging = args.get(position + 1)?;
    let install_root = args.get(position + 2)?;
    Some(crate::updater::desk::run_apply_update(
        std::path::Path::new(staging),
        std::path::Path::new(install_root),
    ))
}

/// Handles `--uninstall-desk <install_root> (--delete-user-data|--keep-user-data)`.
/// Default is keep user data unless `--delete-user-data` is explicit.
fn try_run_uninstall_desk() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let position = args.iter().position(|arg| arg == "--uninstall-desk")?;
    let install_root = args.get(position + 1)?;
    let delete_user_data = args.iter().any(|arg| arg == "--delete-user-data");
    Some(crate::updater::desk::run_uninstall(
        std::path::Path::new(install_root),
        delete_user_data,
    ))
}
