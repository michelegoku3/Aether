#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod core;
mod crack;
mod manifest;
mod providers;
mod steam;
mod steamless;
mod store;
mod updater;
mod util;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // One-time data layout migration: if a legacy `AetherData/lua_backups`
            // folder exists, move its Lua + depotcache manifests into the new
            // centralized `backup` tree and remove the old folder.
            let steam_path = crate::core::settings::SettingsManager::new(&app.handle())
                .load()
                .steam_path;
            match crate::core::migration::migrate_legacy_lua_backups(std::path::Path::new(&steam_path)) {
                Ok(report) => {
                    if report.games > 0 {
                        eprintln!(
                            "[AetherDesk] migrated {} game(s) from lua_backups: {} lua, {} manifest",
                            report.games, report.lua_files, report.manifest_files
                        );
                    }
                }
                Err(error) => eprintln!("[AetherDesk] migration failed: {error}"),
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::settings::validate_hubcap_key,
            commands::settings::get_hubcap_usage,
            commands::crack::pick_crack_files,
            commands::crack::apply_crack,
            commands::antivirus::get_antivirus_exclusion_done,
            commands::antivirus::acknowledge_antivirus_exclusion,
            commands::antivirus::apply_antivirus_exclusion,
            commands::antivirus::open_windows_security,
            commands::antivirus::open_app_folder,
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
