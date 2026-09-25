use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The entire supported AetherDLL installation beside steam.exe.
pub const AETHER_DLL_FILES: [&str; 3] = [
    "AetherCore.dll",
    "AetherPayload.dll",
    "xinput1_4.dll",
];
// Obsolete Steam-root proxy from older AetherDLL builds; removed by migration.
const LEGACY_PROXY: &str = "dwmapi.dll";

pub struct DllInstaller {
    steam_path: PathBuf,
}

impl DllInstaller {
    pub fn new(steam_path: String) -> Self {
        // Normalize at the boundary so quoted/padded-but-valid paths work.
        Self {
            steam_path: PathBuf::from(crate::steam::resolve::normalize_steam_path(&steam_path)),
        }
    }

    /// Validated installation root for every mutating operation. Read-only
    /// probes (`verify_installation`, `count_aether_residuals`) intentionally
    /// stay infallible and keep their own cheap existence checks.
    fn validated_root(&self) -> Result<PathBuf, String> {
        let raw = self.steam_path.to_string_lossy();
        crate::steam::resolve::resolve_steam_path(&raw)
            .map_err(|error| error.message(&raw))
    }

    /// Idempotent migration of the obsolete Steam-root proxy. The caller must
    /// ensure Steam is closed; a locked file fails without being ignored.
    pub fn migrate_legacy_proxy(&self) -> Result<bool, String> {
        let legacy = self.validated_root()?.join(LEGACY_PROXY);
        match fs::remove_file(&legacy) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format_file_operation_error("remove obsolete DLL", &legacy, error)),
        }
    }

    pub fn has_legacy_proxy(&self) -> bool {
        // Unlike Path::exists(), a dangling link or inaccessible path must not
        // be mistaken for a completed migration.
        !matches!(
            fs::symlink_metadata(self.steam_path.join(LEGACY_PROXY)),
            Err(error) if error.kind() == io::ErrorKind::NotFound
        )
    }

    /// A complete installation has the three supported DLLs, not the old proxy.
    pub fn verify_installation(&self) -> bool {
        self.steam_path.is_dir()
            && !self.has_legacy_proxy()
            && AETHER_DLL_FILES
                .iter()
                .all(|file_name| self.steam_path.join(file_name).is_file())
    }

    /// Extracts exactly the three supported DLLs into the Steam root. Rejects
    /// release archives containing any other DLL before changing an installation.
    pub fn install_from_zip(&self, zip_file_path: &Path) -> Result<(), String> {
        self.validated_root()?;

        let file = fs::File::open(zip_file_path)
            .map_err(|e| format!("Failed to open downloaded ZIP: {}", e))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| format!("Invalid ZIP archive format: {}", e))?;

        let mut entries = [None; AETHER_DLL_FILES.len()];
        for i in 0..archive.len() {
            let file = archive.by_index(i).map_err(|e| format!("Invalid ZIP entry: {e}"))?;
            let Some(enclosed) = file.enclosed_name() else {
                return Err("Invalid path in AetherDLL release ZIP".to_string());
            };
            let Some(name) = enclosed.file_name().and_then(|s| s.to_str()) else { continue };
            if let Some(position) = AETHER_DLL_FILES.iter().position(|target| *target == name) {
                if file.is_dir() || entries[position].replace(i).is_some() {
                    return Err(format!("Duplicate or invalid DLL entry in release ZIP: {name}"));
                }
            } else if !file.is_dir() && name.to_ascii_lowercase().ends_with(".dll") {
                return Err(format!("Unsupported DLL in release ZIP: {name}"));
            }
        }
        let missing: Vec<_> = AETHER_DLL_FILES
            .iter()
            .enumerate()
            .filter_map(|(i, name)| entries[i].is_none().then_some(*name))
            .collect();
        if !missing.is_empty() {
            return Err(format!("Incomplete AetherDLL release ZIP: missing {}", missing.join(", ")));
        }

        // Stage every DLL before the first replacement; a corrupt ZIP cannot
        // partially replace a working installation. Steam must be closed so
        // Windows permits renaming the existing DLLs.
        let staged: Vec<_> = AETHER_DLL_FILES
            .iter()
            .map(|name| self.steam_path.join(format!("{name}.aether.tmp")))
            .collect();
        let extraction = (|| -> Result<(), String> {
            for (i, temp_path) in staged.iter().enumerate() {
                let mut input = archive
                    .by_index(entries[i].expect("validated ZIP index"))
                    .map_err(|e| format!("Failed to read {name}: {e}", name = AETHER_DLL_FILES[i]))?;
                let mut output = fs::File::create(temp_path)
                    .map_err(|e| format_file_operation_error("create", temp_path, e))?;
                io::copy(&mut input, &mut output)
                    .map_err(|e| format!("Failed to extract {}: {e}", AETHER_DLL_FILES[i]))?;
                output.sync_all()
                    .map_err(|e| format_file_operation_error("write", temp_path, e))?;
            }
            Ok(())
        })();
        if let Err(error) = extraction {
            for temp in &staged { let _ = fs::remove_file(temp); }
            return Err(error);
        }

        // Back up the whole old set before replacing any file. A locked proxy
        // should fail before installing a new Core, not leave mixed versions.
        let backups: Vec<_> = AETHER_DLL_FILES
            .iter()
            .map(|name| self.steam_path.join(format!("{name}.aether.bak")))
            .collect();
        if backups.iter().any(|path| path.exists()) {
            for temp in &staged { let _ = fs::remove_file(temp); }
            return Err("An earlier AetherDLL backup still exists in the Steam root. Restore or move the *.aether.bak files before retrying.".into());
        }
        let mut backed_up = Vec::new();
        let mut installed = Vec::new();
        let replacement = (|| -> Result<(), String> {
            for (i, name) in AETHER_DLL_FILES.iter().enumerate() {
                let target = self.steam_path.join(name);
                if target.exists() {
                    if !target.is_file() {
                        return Err(format!("DLL target is not a file: {}", target.display()));
                    }
                    fs::rename(&target, &backups[i])
                        .map_err(|e| format_file_operation_error("back up", &target, e))?;
                    backed_up.push(i);
                }
            }
            for (i, name) in AETHER_DLL_FILES.iter().enumerate() {
                let target = self.steam_path.join(name);
                fs::rename(&staged[i], &target)
                    .map_err(|e| format_file_operation_error("install", &target, e))?;
                installed.push(i);
            }
            // Migrate only after the ZIP and all new binaries are in place.
            // A deletion failure rolls back the three managed DLLs below.
            self.migrate_legacy_proxy()?;
            Ok(())
        })();
        if let Err(error) = replacement {
            let mut rollback_errors = Vec::new();
            for i in installed.into_iter().rev() {
                let target = self.steam_path.join(AETHER_DLL_FILES[i]);
                if let Err(e) = fs::remove_file(&target) {
                    rollback_errors.push(format!("{}: {e}", target.display()));
                }
            }
            for i in backed_up.into_iter().rev() {
                let target = self.steam_path.join(AETHER_DLL_FILES[i]);
                if let Err(e) = fs::rename(&backups[i], &target) {
                    rollback_errors.push(format!("{}: {e}", target.display()));
                }
            }
            for temp in &staged { let _ = fs::remove_file(temp); }
            return if rollback_errors.is_empty() { Err(error) } else {
                Err(format!("{error}; rollback incomplete: {}", rollback_errors.join(", ")))
            };
        }
        for backup in &backups {
            if backup.exists() {
                fs::remove_file(backup)
                    .map_err(|e| format_file_operation_error("delete obsolete backup", backup, e))?;
            }
        }
        Ok(())
    }

    /// Removes the three supported DLLs and the obsolete Steam-root proxy.
    pub fn uninstall(&self) -> Result<(), String> {
        self.validated_root()?;
        let mut deleted_count = usize::from(self.migrate_legacy_proxy()?);

        for file_name in AETHER_DLL_FILES {
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
    /// Targets are the single source of truth shared with [`Self::count_aether_residuals`].
    pub fn reset_aether_files(&self) -> Result<usize, String> {
        self.validated_root()?;
        let mut removed = usize::from(self.migrate_legacy_proxy()?);

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

    /// Counts residual Aether artifacts (files, dirs, non-empty depotcache)
    /// and the obsolete proxy until the migration removes it.
    /// Used by the portable Uninstall flow to decide whether to prompt for a
    /// Steam clean. Does not require Steam to be closed.
    pub fn count_aether_residuals(&self) -> usize {
        if !self.steam_path.exists() {
            return 0;
        }

        let mut count = 0;
        for path in self.aether_files() {
            if path.exists() {
                count += 1;
            }
        }
        for path in self.aether_directories() {
            if path.exists() {
                count += 1;
            }
        }
        count += self.depotcache_entry_count();
        if self.has_legacy_proxy() {
            count += 1;
        }
        count
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
            // Includes desk_path.cfg pointer + any legacy aethercore.toml bridge.
            self.steam_path.join("aethercore"),
            self.steam_path.join("config").join("stplug-in"),
        ]
    }

    fn depotcache_entry_count(&self) -> usize {
        let depotcache = self.steam_path.join("depotcache");
        if !depotcache.is_dir() {
            return 0;
        }
        fs::read_dir(&depotcache)
            .map(|entries| entries.flatten().count())
            .unwrap_or(0)
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
