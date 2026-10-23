use std::fs;
use std::path::PathBuf;
use crate::core::paths::LocalAppPaths;

const COMPONENT_VERSION_DIR: &str = "component_versions";
const AETHER_DLL_VERSION_FILE: &str = "aetherdll_version.txt";

pub struct AppStorage {
    app_dir: PathBuf,
}

impl AppStorage {
    pub fn new(app: &tauri::AppHandle) -> Self {
        Self {
            app_dir: LocalAppPaths::data_root_for_app(app),
        }
    }

    pub fn read_aether_dll_version(&self) -> Option<String> {
        Self::read_version_file(self.aether_dll_version_path())
    }

    pub fn write_aether_dll_version(&self, version: &str) -> Result<(), String> {
        let path = self.aether_dll_version_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create component version directory next to AetherDesk: {}", e))?;
        }

        let temp_path = path.with_extension("tmp");
        fs::write(&temp_path, version)
            .map_err(|e| format!("Failed to write temporary AetherDLL version: {}", e))?;
        fs::rename(&temp_path, &path)
            .map_err(|e| format!("Failed to apply AetherDLL version: {}", e))
    }

    pub fn remove_aether_dll_version(&self) {
        let _ = fs::remove_file(self.aether_dll_version_path());
    }

    fn read_version_file(path: impl Into<PathBuf>) -> Option<String> {
        fs::read_to_string(path.into())
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn aether_dll_version_path(&self) -> PathBuf {
        self.app_dir.join(COMPONENT_VERSION_DIR).join(AETHER_DLL_VERSION_FILE)
    }
}
