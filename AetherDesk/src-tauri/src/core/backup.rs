// Centralized per-game backup layout under AetherData.
//
// Target layout (single source of truth for everything a game may need to
// restore or re-apply later):
//
//   <AetherData>/backup/<app_id>/
//       lua/        <app_id>.lua + any bundled Steam .manifest files
//       original/   original game files replaced by the crack, plus a text file
//                   listing every file the crack adds (crack "inventory")
//       crack/      the crack files themselves (reused if still working)
//
// This module owns creating that structure and writing into it. It is a pure
// filesystem service: it takes no AppHandle and knows nothing about Steam, so
// it stays small, testable and decoupled from the Tauri command layer.
use crate::core::paths::LocalAppPaths;
use crate::manifest::package::ManifestPackageFile;
use std::fs;
use std::path::{Path, PathBuf};

const BACKUP_ROOT: &str = "backup";
const LUA_SUBDIR: &str = "lua";
const ORIGINAL_SUBDIR: &str = "original";
const CRACK_SUBDIR: &str = "crack";

pub struct GameBackup {
    root: PathBuf,
}

impl GameBackup {
    /// Build a `GameBackup` handle for an app, creating the `backup/<app_id>/`
    /// tree (with `lua`, `original` and `crack` sub-folders) on first use.
    pub fn for_app(app_id: u32) -> Result<Self, String> {
        let root = LocalAppPaths::data_root()
            .join(BACKUP_ROOT)
            .join(app_id.to_string());

        for sub in [LUA_SUBDIR, ORIGINAL_SUBDIR, CRACK_SUBDIR] {
            fs::create_dir_all(root.join(sub)).map_err(|error| {
                format!("Failed to create backup folder {}: {}", root.join(sub).display(), error)
            })?;
        }

        Ok(Self { root })
    }

    pub fn lua_dir(&self) -> PathBuf {
        self.root.join(LUA_SUBDIR)
    }

    pub fn original_dir(&self) -> PathBuf {
        self.root.join(ORIGINAL_SUBDIR)
    }

    pub fn crack_dir(&self) -> PathBuf {
        self.root.join(CRACK_SUBDIR)
    }

    /// Path of the crack inventory file (`original/crack_<app_id>.txt`).
    pub fn crack_inventory_path(&self, app_id: u32) -> PathBuf {
        self.original_dir().join(format!("crack_{}.txt", app_id))
    }

    /// Open an existing backup tree without creating folders.
    /// Returns `None` when `backup/<app_id>` does not exist yet.
    pub fn open_existing(app_id: u32) -> Option<Self> {
        let root = LocalAppPaths::data_root()
            .join(BACKUP_ROOT)
            .join(app_id.to_string());
        if root.is_dir() {
            Some(Self { root })
        } else {
            None
        }
    }

    /// True when `backup/<app_id>/crack/` contains at least one file.
    pub fn has_saved_crack(&self) -> bool {
        dir_has_files(&self.crack_dir())
    }

    /// Recursively list files under `crack/` as paths relative to the crack dir
    /// (same layout as game-relative paths used when the crack was applied).
    pub fn list_saved_crack_files(&self) -> Result<Vec<String>, String> {
        let crack_dir = self.crack_dir();
        if !crack_dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut files = Vec::new();
        collect_relative_files(&crack_dir, &crack_dir, &mut files)?;
        files.sort();
        Ok(files)
    }

    /// Read the crack inventory (game-relative paths, one per line).
    pub fn read_crack_inventory(&self, app_id: u32) -> Result<Vec<String>, String> {
        let path = self.crack_inventory_path(app_id);
        if !path.is_file() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&path).map_err(|error| {
            format!("Failed to read crack inventory {}: {}", path.display(), error)
        })?;
        Ok(content
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| line.to_string())
            .collect())
    }

    /// Clear the crack inventory after the applied crack has been removed from the game.
    pub fn clear_crack_inventory(&self, app_id: u32) -> Result<(), String> {
        let path = self.crack_inventory_path(app_id);
        if path.exists() {
            fs::remove_file(&path).map_err(|error| {
                format!("Failed to remove crack inventory {}: {}", path.display(), error)
            })?;
        }
        Ok(())
    }

    /// Persist the Lua and any bundled Steam `.manifest` files for a game.
    ///
    /// This is the central "Lua backup" step: it is called every time a Lua is
    /// downloaded/installed, so the game's Lua manifest source is always kept.
    /// Writes are atomic (temp file + rename) to avoid a partially-written file
    /// if the app is interrupted.
    pub fn backup_lua_artifacts(
        &self,
        app_id: u32,
        lua_content: &str,
        manifest_files: &[ManifestPackageFile],
    ) -> Result<(), String> {
        let lua_path = self.lua_dir().join(format!("{}.lua", app_id));
        write_atomic(&lua_path, lua_content.as_bytes())?;

        for manifest in manifest_files {
            let manifest_path = self.lua_dir().join(&manifest.file_name);
            write_atomic(&manifest_path, &manifest.bytes)?;
        }

        Ok(())
    }
}

/// Write bytes to `path` atomically via a temp file + rename.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp_path = PathBuf::from(format!("{}.tmp", path.display()));

    fs::write(&temp_path, bytes)
        .map_err(|error| format!("Failed to write temporary file {}: {}", temp_path.display(), error))?;
    fs::rename(&temp_path, path)
        .map_err(|error| format!("Failed to finalize file {}: {}", path.display(), error))
}

fn dir_has_files(dir: &Path) -> bool {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                return true;
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    false
}

fn collect_relative_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|error| format!("Failed to read folder {}: {}", dir.display(), error))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("Failed to read folder entry: {}", error))?;
        let path = entry.path();
        if path.is_dir() {
            collect_relative_files(root, &path, out)?;
        } else if path.is_file() {
            let rel = path.strip_prefix(root).map_err(|_| {
                format!(
                    "Internal error: file {} is outside {}",
                    path.display(),
                    root.display()
                )
            })?;
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}
