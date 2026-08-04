use std::fs;
use std::path::PathBuf;
use crate::local_app_paths::LocalAppPaths;

const COMPONENT_VERSION_DIR: &str = "component_versions";
const AETHER_DLL_VERSION_FILE: &str = "aetherdll_version.txt";

pub struct AppStorage {
    app_dir: PathBuf,
    legacy_app_dir: Option<PathBuf>,
}

impl AppStorage {
    pub fn new(app: &tauri::AppHandle) -> Self {
        Self {
            app_dir: LocalAppPaths::data_root(),
            legacy_app_dir: LocalAppPaths::legacy_app_data_dir(app),
        }
    }

    pub fn read_aether_dll_version(&self) -> Option<String> {
        if let Some(version) = Self::read_version_file(self.aether_dll_version_path()) {
            return Some(version);
        }

        let legacy_path = self.legacy_aether_dll_version_path()?;
        let version = Self::read_version_file(&legacy_path)?;
        let _ = self.write_aether_dll_version(&version);
        let _ = fs::remove_file(legacy_path);
        Some(version)
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
        if let Some(legacy_path) = self.legacy_aether_dll_version_path() {
            let _ = fs::remove_file(legacy_path);
        }
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

    fn legacy_aether_dll_version_path(&self) -> Option<PathBuf> {
        self.legacy_app_dir
            .as_ref()
            .map(|dir| dir.join(COMPONENT_VERSION_DIR).join(AETHER_DLL_VERSION_FILE))
    }
}
