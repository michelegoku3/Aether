#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod app_storage;
mod commands;
mod dll_installer;
mod download_orchestrator;
mod drm_detector;
mod github_updater;
mod hubcap_client;
mod local_app_paths;
mod lua_manifest_pins;
mod manifest_package;
mod oureveryday_client;
mod settings;
mod steam_app_names;
mod steam_compat;
mod steam_library;
mod steam_store;
mod steam_update_guard;
mod steamless;
mod store_search_cache;
mod store_service;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::settings::validate_hubcap_key,
            commands::settings::get_hubcap_usage,
            commands::crack::pick_crack_files,
            commands::crack::apply_crack,
            commands::store::search_store,
            commands::store::check_denuvo_bulk,
            commands::store::trigger_hubcap_download,
            commands::store::prepare_specific_version_download,
            commands::library::get_installed_library_games,
            commands::library::warm_library_game_cache,
            commands::library::open_steamdb_depots,
            commands::home_links::open_home_resource,
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
