use crate::core::paths::LocalAppPaths;
use crate::external_tools::bundle::ToolBundleLocator;
use std::path::{Path, PathBuf};

const STEAMLESS_CLI: &str = "Steamless.CLI.exe";
const STEAMLESS_RESOURCE_DIR: &str = "ExternalTools/Steamless";

#[derive(Debug, Clone)]
pub struct SteamlessTool {
    pub cli_path: PathBuf,
    pub working_dir: PathBuf,
}

pub struct SteamlessToolLocator {
    inner: ToolBundleLocator,
}

impl SteamlessToolLocator {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self {
            inner: ToolBundleLocator::new(
                app,
                STEAMLESS_RESOURCE_DIR,
                LocalAppPaths::steamless_dir(),
                Self::is_valid_steamless_dir,
            ),
        }
    }

    pub fn locate(&self) -> Result<SteamlessTool, String> {
        let bundle = self.inner.locate().map_err(|_| {
            format!(
                "Steamless was not found. Expected bundled tool at: {}",
                LocalAppPaths::steamless_dir().join(STEAMLESS_CLI).display()
            )
        })?;

        Ok(SteamlessTool {
            cli_path: bundle.dir.join(STEAMLESS_CLI),
            working_dir: bundle.dir,
        })
    }

    fn is_valid_steamless_dir(dir: &Path) -> bool {
        dir.join(STEAMLESS_CLI).is_file() && dir.join("Plugins").is_dir()
    }
}
