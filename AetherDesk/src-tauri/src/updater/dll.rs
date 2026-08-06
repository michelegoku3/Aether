use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The 3 binary files that make up an AetherDLL installation in the Steam directory.
/// Single source of truth for the names: used by the installer, uninstaller, reset
/// and by the PE version-resource reader (`updater::dll_version`).
pub const AETHER_DLL_FILES: [&str; 3] = ["AetherCore.dll", "AetherPayload.dll", "dwmapi.dll"];

pub struct DllInstaller {
    steam_path: PathBuf,
}

impl DllInstaller {
    pub fn new(steam_path: String) -> Self {
        Self {
            steam_path: PathBuf::from(steam_path),
        }
    }

    /// Verifies if the 3 target DLL files exist in the main Steam directory
    pub fn verify_installation(&self) -> bool {
        if !self.steam_path.exists() {
            return false;
        }

        AETHER_DLL_FILES
            .iter()
            .all(|file_name| self.steam_path.join(file_name).exists())
    }

    /// Takes a downloaded release ZIP file and extracts AetherCore.dll, AetherPayload.dll, and dwmapi.dll
    /// directly into the main Steam directory, overwriting any previous versions.
    pub fn install_from_zip(&self, zip_file_path: &Path) -> Result<(), String> {
        if !self.steam_path.exists() {
            return Err("Steam installation path does not exist".to_string());
        }

        let file = fs::File::open(zip_file_path)
            .map_err(|e| format!("Failed to open downloaded ZIP: {}", e))?;

        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| format!("Invalid ZIP archive format: {}", e))?;

        let mut extracted_count = 0;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i).unwrap();
            let file_name = match file.enclosed_name() {
                Some(name) => name.file_name().unwrap_or_default().to_string_lossy().to_string(),
                None => continue,
            };

            // We only extract the target DLL files we care about (ignoring directory structures)
            if AETHER_DLL_FILES.contains(&file_name.as_str()) {
                let target_path = self.steam_path.join(&file_name);
                let temp_path = target_path.with_extension("tmp");

                let mut outfile = fs::File::create(&temp_path)
                    .map_err(|e| format_file_operation_error("create", &target_path, e))?;

                io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file contents: {}", e))?;

                // Atomic replacement
                fs::rename(&temp_path, &target_path)
                    .map_err(|e| format_file_operation_error("replace", &target_path, e))?;

                extracted_count += 1;
            }
        }

        if extracted_count < AETHER_DLL_FILES.len() {
            return Err(format!(
                "Failed to locate all {} required DLL files in the ZIP. Found only {}.",
                AETHER_DLL_FILES.len(),
                extracted_count
            ));
        }

        Ok(())
    }

    /// Removes AetherCore.dll, AetherPayload.dll, and dwmapi.dll from the Steam directory
    pub fn uninstall(&self) -> Result<(), String> {
        if !self.steam_path.exists() {
            return Err("Steam installation path does not exist".to_string());
        }

        let files_to_delete = AETHER_DLL_FILES;
        let mut deleted_count = 0;

        for file_name in files_to_delete {
            let file_path = self.steam_path.join(file_name);
            if file_path.exists() {
                fs::remove_file(&file_path)
                    .map_err(|e| format_file_operation_error("delete", &file_path, e))?;
                deleted_count += 1;
            }
        }

        if deleted_count == 0 {
            return Err("AetherDLL files were not found in the target Steam directory.".to_string());
        }

        Ok(())
    }

    /// Removes every known file/folder created by Aether inside the Steam directory.
    pub fn reset_aether_files(&self) -> Result<usize, String> {
        if !self.steam_path.exists() {
            return Err("Steam installation path does not exist".to_string());
        }

        let mut removed = 0;

        for file_path in self.aether_files() {
            if file_path.exists() {
                fs::remove_file(&file_path)
                    .map_err(|e| format_file_operation_error("delete", &file_path, e))?;
                removed += 1;
            }
        }

        for dir_path in self.aether_directories() {
            if dir_path.exists() {
                fs::remove_dir_all(&dir_path)
                    .map_err(|e| format_file_operation_error("delete folder", &dir_path, e))?;
                removed += 1;
            }
        }

        removed += self.clear_depotcache_contents()?;

        Ok(removed)
    }

    fn aether_files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = AETHER_DLL_FILES
            .iter()
            .map(|file_name| self.steam_path.join(file_name))
            .collect();
        files.push(self.steam_path.join("AetherDLL_version.txt"));
        files.push(self.steam_path.join("steam.cfg"));
        files.push(self.steam_path.join("bin").join("acoverlay.dll"));

        files
    }

    fn aether_directories(&self) -> Vec<PathBuf> {
        vec![
            self.steam_path.join("aethercore"),
            self.steam_path.join("config").join("stplug-in"),
        ]
    }

    fn clear_depotcache_contents(&self) -> Result<usize, String> {
        let depotcache = self.steam_path.join("depotcache");
        if !depotcache.is_dir() {
            return Ok(0);
        }

        let mut removed = 0;
        for entry in fs::read_dir(&depotcache)
            .map_err(|e| format_file_operation_error("read folder", &depotcache, e))?
        {
            let entry = entry.map_err(|e| format!("Failed to read depotcache entry: {}", e))?;
            let path = entry.path();
            if path.is_dir() {
                fs::remove_dir_all(&path)
                    .map_err(|e| format_file_operation_error("delete folder", &path, e))?;
            } else {
                fs::remove_file(&path)
                    .map_err(|e| format_file_operation_error("delete", &path, e))?;
            }
            removed += 1;
        }

        Ok(removed)
    }
}

fn format_file_operation_error(action: &str, path: &Path, error: std::io::Error) -> String {
    format!(
        "Failed to {} {}. If Steam is running, close Steam completely and try again. Details: {}",
        action,
        path.display(),
        error
    )
}
