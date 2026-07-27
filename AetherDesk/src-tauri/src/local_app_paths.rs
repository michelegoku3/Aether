use std::path::PathBuf;
use tauri::Manager;

const LOCAL_DATA_DIR_NAME: &str = "AetherData";

/// Centralized path resolver for AetherDesk-owned files.
///
/// User preference: keep AetherDesk data next to the installed executable, i.e. the
/// folder opened by "Open file location". Therefore the primary data root is:
///
/// `<folder-containing-aether_desk.exe>/AetherData/`
///
/// Legacy AppData paths are exposed only for one-time migration/cleanup from older builds.
pub struct LocalAppPaths;

impl LocalAppPaths {
    pub fn data_root() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|parent| parent.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
            .join(LOCAL_DATA_DIR_NAME)
    }

    pub fn config_dir() -> PathBuf {
        Self::data_root().join("config")
    }

    pub fn legacy_app_data_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
        app.path().app_data_dir().ok()
    }

    pub fn legacy_app_config_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
        app.path().app_config_dir().ok()
    }
}
