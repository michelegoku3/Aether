//! Location and idempotent installation of external tool bundles.
//!
//! Generalization of the Steamless locator (`steamless/tool_locator.rs`):
//! an external tool is looked up first in its installed directory, then
//! copied from the bundle vendored in the repo/resources. Each tool provides
//! its own validator (e.g. "Steamless.CLI.exe + Plugins/ exists"), so this
//! class knows nothing about individual tools (low coupling).

use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

/// An external tool bundle, located and ready to use.
#[derive(Debug, Clone)]
pub struct ToolBundle {
    /// Tool root directory (e.g. `<install>/ExternalTools/Steamless`).
    pub dir: PathBuf,
}

/// Locates (and when needed installs) an external tool's bundle.
pub struct ToolBundleLocator {
    app: tauri::AppHandle,
    /// Bundled resources subfolder, e.g. `"ExternalTools/Steamless"`.
    resource_subdir: &'static str,
    /// Installed destination directory (typically from `LocalAppPaths`).
    installed_dir: PathBuf,
    /// Predicate deciding whether a directory holds a valid bundle.
    validator: Box<dyn Fn(&Path) -> bool>,
}

impl ToolBundleLocator {
    pub fn new(
        app: tauri::AppHandle,
        resource_subdir: &'static str,
        installed_dir: PathBuf,
        validator: impl Fn(&Path) -> bool + 'static,
    ) -> Self {
        Self {
            app,
            resource_subdir,
            installed_dir,
            validator: Box::new(validator),
        }
    }

    /// Returns a valid bundle, installing it from the bundled source when
    /// the installed directory is missing or invalid.
    pub fn locate(&self) -> Result<ToolBundle, String> {
        if self.is_valid(&self.installed_dir) {
            return Ok(ToolBundle {
                dir: self.installed_dir.clone(),
            });
        }

        if let Some(source_dir) = self.bundled_source_dir() {
            self.copy_bundle(&source_dir, &self.installed_dir)?;
            if self.is_valid(&self.installed_dir) {
                return Ok(ToolBundle {
                    dir: self.installed_dir.clone(),
                });
            }
        }

        Err(format!(
            "Tool bundle was not found. Expected installed directory: {}",
            self.installed_dir.display()
        ))
    }

    fn is_valid(&self, dir: &Path) -> bool {
        (self.validator)(dir)
    }

    fn bundled_source_dir(&self) -> Option<PathBuf> {
        let mut candidates = Vec::new();

        if let Ok(resource_dir) = self.app.path().resource_dir() {
            candidates.push(resource_dir.join(self.resource_subdir));
        }

        candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(self.resource_subdir));

        candidates
            .into_iter()
            .find(|candidate| self.is_valid(candidate))
    }

    /// Copies the vendored bundle into the installed directory. Idempotent but
    /// destructive: an incomplete/invalid installed directory is removed and
    /// recreated from source (same behavior as the old Steamless locator).
    fn copy_bundle(&self, source: &Path, destination: &Path) -> Result<(), String> {
        if destination.exists() {
            let _ = fs::remove_dir_all(destination);
        }

        copy_dir_all(source, destination).map_err(|error| {
            format!(
                "Failed to install bundled tool from {} to {}: {}",
                source.display(),
                destination.display(),
                error
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
