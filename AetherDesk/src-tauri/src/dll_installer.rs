use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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

        let core_exists = self.steam_path.join("AetherCore.dll").exists();
        let payload_exists = self.steam_path.join("AetherPayload.dll").exists();
        let proxy_exists = self.steam_path.join("dwmapi.dll").exists();

        core_exists && payload_exists && proxy_exists
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

            // We only extract the 3 DLL files we care about (ignoring directory structures)
            if file_name == "AetherCore.dll" || file_name == "AetherPayload.dll" || file_name == "dwmapi.dll" {
                let target_path = self.steam_path.join(&file_name);
                let temp_path = target_path.with_extension("tmp");

                let mut outfile = fs::File::create(&temp_path)
                    .map_err(|e| format!("Failed to create target file {:?}: {}", target_path, e))?;

                io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file contents: {}", e))?;

                // Atomic replacement
                fs::rename(&temp_path, &target_path)
                    .map_err(|e| format!("Failed to apply file replacement: {}", e))?;

                extracted_count += 1;
            }
        }

        if extracted_count < 3 {
            return Err(format!(
                "Failed to locate all 3 required DLL files in the ZIP. Found only {} of 3.",
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

        let files_to_delete = vec!["AetherCore.dll", "AetherPayload.dll", "dwmapi.dll"];
        let mut deleted_count = 0;

        for file_name in files_to_delete {
            let file_path = self.steam_path.join(file_name);
            if file_path.exists() {
                fs::remove_file(&file_path)
                    .map_err(|e| format!("Failed to delete file {:?}: {}", file_path, e))?;
                deleted_count += 1;
            }
        }

        if deleted_count == 0 {
            return Err("AetherDLL files were not found in the target Steam directory.".to_string());
        }

        Ok(())
    }
}
