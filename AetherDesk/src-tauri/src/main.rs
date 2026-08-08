#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod core;
mod crack;
mod game_info;
mod manifest;
mod providers;
mod steam;
mod steamless;
mod store;
mod updater;
mod util;

#[cfg(test)]
mod tests;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // All startup migrations live in one place: legacy settings move,
            // obsolete data-folder cleanup, and the lua_backups → backup data
            // layout migration. Each step is idempotent and failure-tolerant.
            crate::core::migration::run_startup_migrations(&app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::settings::validate_hubcap_key,
            commands::settings::get_hubcap_usage,
            commands::settings::clear_app_caches,
            commands::custom_css::get_custom_css,
            commands::custom_css::get_custom_css_path,
            commands::custom_css::get_personal_wallpaper_path,
            commands::custom_css::get_personal_wallpaper_data_uri,
            commands::custom_css::ensure_custom_css,
            commands::custom_css::open_custom_css_folder,
            commands::crack::pick_crack_files,
            commands::crack::apply_crack,
            commands::antivirus::get_antivirus_exclusion_done,
            commands::antivirus::acknowledge_antivirus_exclusion,
            commands::antivirus::apply_antivirus_exclusion,
            commands::antivirus::open_windows_security,
            commands::antivirus::open_app_folder,
            commands::store::search_store,
            commands::store::get_cached_store_search,
            commands::store::check_denuvo_bulk,
            commands::store::trigger_hubcap_download,
            commands::store::trigger_ryuu_download,
            commands::store::prepare_specific_version_download,
            commands::store::prepare_ryuu_specific_version_download,
            commands::library::get_installed_library_games,
            commands::library::warm_library_game_cache,
            commands::library::open_steamdb_depots,
            commands::home_links::open_home_resource,
            commands::game_info::get_game_info,
            commands::steamless::pick_and_run_steamless,
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
            commands::aether_dll::check_aether_dll_update,
            commands::aether_dll::install_aether_dll,
            commands::aether_dll::uninstall_aether_dll,
            commands::aether_dll::reset_aether_steam_path,
            commands::aether_desk::check_aether_desk_update,
            commands::aether_desk::install_aether_desk_update,
            commands::aether_desk::uninstall_aether_desk,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
