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
use crate::local_app_paths::LocalAppPaths;
use crate::manifest_package::ManifestPackageFile;
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
