use std::path::PathBuf;
use tauri::Manager;

const LOCAL_DATA_DIR_NAME: &str = "AetherData";

/// Centralized path resolver for AetherDesk.
///
/// ## Portable distribution (ZIP)
/// AetherDesk is a self-contained portable folder: executable, tools and data
/// all live together. The whole Aether folder holds everything inside itself —
/// nothing in `AppData\Local` or `AppData\Roaming`. Solution: both the
/// executable and `AetherData` live inside the same installation folder
/// (`install_root/`). Layout:
///
///   <where-the-user-unpacks-it>/AetherDesk\
///     ├─ AetherDesk.exe
///     ├─ ExternalTools\Steamless\...
///     └─ AetherData\
///         ├─ config\themes\*.css
///         ├─ config\wallpapers\*.jpg
///         ├─ config\settings.json
///         └─ backup\...
///
/// Old locations (migrated only for users coming from an old install):
/// Old Program Files location: `<exe_parent>\AetherData`, from when the app
/// lived in `C:\Program Files`.
/// Old Roaming location: `%APPDATA%\com.aether.desk` (Roaming).
/// Both are automatically migrated on first launch to `install_root/AetherData`.
pub struct LocalAppPaths;

impl LocalAppPaths {
    /// Folder containing the executable.
    /// With `currentUser` → `%LOCALAPPDATA%\AetherDesk\`
    /// With `Program Files` (old installs) → `C:\Program Files\AetherDesk\`
    pub fn install_root() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|parent| parent.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    /// Primary and only path: `<install_root>/AetherData`.
    /// It is the only location used from now on.
    pub fn data_root() -> PathBuf {
        Self::install_root().join(LOCAL_DATA_DIR_NAME)
    }

    /// Compatibility alias for code that used to take `&AppHandle`.
    /// Now ignores `app` and always returns `install_root/AetherData` (unified).
    pub fn data_root_for_app(_app: &tauri::AppHandle) -> PathBuf {
        Self::data_root()
    }

    /// Legacy Roaming from the v2 fix: `%APPDATA%\com.aether.desk`
    /// Kept only for back-migration to the unified install.
    pub fn legacy_roaming_data_root() -> PathBuf {
        if let Some(base) = dirs::data_dir() {
            return base.join("com.aether.desk");
        }
        Self::install_root().join(LOCAL_DATA_DIR_NAME)
    }

    /// Directory dei tool esterni — resta accanto all'exe (read-only bundled).
    pub fn external_tools_dir() -> PathBuf {
        Self::install_root().join("ExternalTools")
    }

    pub fn steamless_dir() -> PathBuf {
        Self::external_tools_dir().join("Steamless")
    }

    /// Directory del bundle UCOnline2 (emulatore Steamworks + plugin).
    pub fn uco2_dir() -> PathBuf {
        Self::external_tools_dir().join("UCOnline2")
    }

    pub fn config_dir() -> PathBuf {
        Self::data_root().join("config")
    }

    /// State directory (uc_online2.json and other application states).
    pub fn state_dir() -> PathBuf {
        Self::data_root().join("state")
    }

    /// Per-game backup root (`<AetherData>/backup`).
    pub fn backup_root() -> PathBuf {
        Self::data_root().join("backup")
    }

    pub fn temp_dir() -> PathBuf {
        Self::data_root().join("temp")
    }

    pub fn legacy_app_data_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
        app.path().app_data_dir().ok()
    }

    pub fn legacy_app_config_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
        app.path().app_config_dir().ok()
    }
}
