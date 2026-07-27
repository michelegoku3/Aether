#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use crate::manifest_package::ManifestPackageFile;

pub struct SteamCompat {
    steam_path: PathBuf,
}

impl SteamCompat {
    pub fn new(steam_path: String) -> Self {
        Self {
            steam_path: PathBuf::from(steam_path),
        }
    }

    /// Returns the path to Steam's main plugin directory
    pub fn get_plugin_dir(&self) -> PathBuf {
        self.steam_path.join("config").join("stplug-in")
    }

    /// Returns the path to Steam's main depotcache directory (where manifests live)
    pub fn get_depotcache_dir(&self) -> PathBuf {
        self.steam_path.join("depotcache")
    }

    /// Safely writes the Lua config to the stplug-in directory
    pub fn install_lua_config(&self, app_id: u32, content: &str) -> Result<(), String> {
        let plugin_dir = self.get_plugin_dir();
        if !plugin_dir.exists() {
            fs::create_dir_all(&plugin_dir)
                .map_err(|e| format!("Failed to create plugin directory: {}", e))?;
        }

        let target_path = plugin_dir.join(format!("{}.lua", app_id));
        let temp_path = target_path.with_extension("tmp");

        // Keep a non-.lua backup before overwriting. This is intentionally not named
        // *.lua so LumaCore/AetherDLL will not try to load it as another plugin file.
        if target_path.exists() {
            let backup_path = target_path.with_extension("lua.bak");
            let _ = fs::copy(&target_path, backup_path);
        }

        fs::write(&temp_path, content)
            .map_err(|e| format!("Failed to write plugin Lua: {}", e))?;

        fs::rename(&temp_path, &target_path)
            .map_err(|e| format!("Failed to install plugin Lua: {}", e))?;

        // Defensive verification: this layer must be a pure writer and must never
        // transform Lua content. If the installed file differs, stop immediately.
        let installed = fs::read_to_string(&target_path)
            .map_err(|e| format!("Failed to verify installed plugin Lua: {}", e))?;
        if installed != content {
            return Err("Installed Lua differs from downloaded Lua; refusing to continue.".to_string());
        }

        Ok(())
    }

    pub fn read_lua_config(&self, app_id: u32) -> Result<String, String> {
        let path = self.get_plugin_dir().join(format!("{}.lua", app_id));
        fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read plugin Lua {}: {}", path.display(), e))
    }

    /// Safely writes Steam depot .manifest files to Steam/depotcache.
    pub fn install_manifest_files(&self, manifests: &[ManifestPackageFile]) -> Result<usize, String> {
        if manifests.is_empty() {
            return Ok(0);
        }

        let depotcache_dir = self.get_depotcache_dir();
        fs::create_dir_all(&depotcache_dir)
            .map_err(|e| format!("Failed to create depotcache directory: {}", e))?;

        let mut installed = 0usize;
        for manifest in manifests {
            let Some(file_name) = Path::new(&manifest.file_name).file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !file_name.to_ascii_lowercase().ends_with(".manifest") {
                continue;
            }

            let target_path = depotcache_dir.join(file_name);
            let temp_path = target_path.with_extension("tmp");
            fs::write(&temp_path, &manifest.bytes)
                .map_err(|e| format!("Failed to write temporary manifest {}: {}", file_name, e))?;
            fs::rename(&temp_path, &target_path)
                .map_err(|e| format!("Failed to install manifest {}: {}", file_name, e))?;
            installed += 1;
        }

        Ok(installed)
    }

    /// Safely writes a decryption manifest .acf file into a steamapps library folder
    pub fn write_acf_manifest(&self, library_folder: String, app_id: u32, acf_content: &str) -> Result<(), String> {
        let library_dir = Path::new(&library_folder);
        if !library_dir.exists() {
            return Err("Library folder does not exist".to_string());
        }

        let target_path = library_dir.join("steamapps").join(format!("appmanifest_{}.acf", app_id));
        let temp_path = target_path.with_extension("tmp");

        fs::create_dir_all(target_path.parent().unwrap())
            .map_err(|e| format!("Failed to create steamapps folder: {}", e))?;

        fs::write(&temp_path, acf_content)
            .map_err(|e| format!("Failed to write temp ACF: {}", e))?;

        fs::rename(&temp_path, &target_path)
            .map_err(|e| format!("Failed to apply ACF file: {}", e))?;

        Ok(())
    }
}
