use crate::core::paths::LocalAppPaths;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

const STEAMLESS_CLI: &str = "Steamless.CLI.exe";
const STEAMLESS_RESOURCE_DIR: &str = "ExternalTools/Steamless";

#[derive(Debug, Clone)]
pub struct SteamlessTool {
    pub cli_path: PathBuf,
    pub working_dir: PathBuf,
}

pub struct SteamlessToolLocator {
    app: tauri::AppHandle,
}

impl SteamlessToolLocator {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }

    pub fn locate(&self) -> Result<SteamlessTool, String> {
        let installed_dir = LocalAppPaths::steamless_dir();
        if self.is_valid_steamless_dir(&installed_dir) {
            return Ok(Self::tool_from_dir(installed_dir));
        }

        if let Some(source_dir) = self.bundled_source_dir() {
            self.copy_steamless_dir(&source_dir, &installed_dir)?;
            if self.is_valid_steamless_dir(&installed_dir) {
                return Ok(Self::tool_from_dir(installed_dir));
            }
        }

        Err(format!(
            "Steamless was not found. Expected bundled tool at: {}",
            installed_dir.join(STEAMLESS_CLI).display()
        ))
    }

    fn bundled_source_dir(&self) -> Option<PathBuf> {
        let mut candidates = Vec::new();

        if let Ok(resource_dir) = self.app.path().resource_dir() {
            candidates.push(resource_dir.join(STEAMLESS_RESOURCE_DIR));
        }

        candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(STEAMLESS_RESOURCE_DIR));

        candidates
            .into_iter()
            .find(|candidate| self.is_valid_steamless_dir(candidate))
    }

    fn is_valid_steamless_dir(&self, dir: &Path) -> bool {
        dir.join(STEAMLESS_CLI).is_file() && dir.join("Plugins").is_dir()
    }

    fn tool_from_dir(dir: PathBuf) -> SteamlessTool {
        SteamlessTool {
            cli_path: dir.join(STEAMLESS_CLI),
            working_dir: dir,
        }
    }

    fn copy_steamless_dir(&self, source: &Path, destination: &Path) -> Result<(), String> {
        if destination.exists() {
            let _ = fs::remove_dir_all(destination);
        }

        copy_dir_all(source, destination).map_err(|e| {
            format!(
                "Failed to install bundled Steamless from {} to {}: {}",
                source.display(),
                destination.display(),
                e
            )
        })
    }
}

fn copy_dir_all(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;

    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }

    Ok(())
}
