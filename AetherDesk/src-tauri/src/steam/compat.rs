use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use crate::manifest::package::ManifestPackageFile;

/// Prevents concurrent package completions from colliding on the shared
/// `<depotcache>/<name>.manifest.tmp` path. Network generation is already
/// deduplicated separately; this protects the final local commit as well.
fn manifest_install_gate() -> &'static Mutex<()> {
    static GATE: OnceLock<Mutex<()>> = OnceLock::new();
    GATE.get_or_init(|| Mutex::new(()))
}

#[derive(Clone)]
pub struct SteamCompat {
    steam_path: PathBuf,
}

impl SteamCompat {
    pub fn new(steam_path: String) -> Self {
        // Normalize at the boundary so quoted/padded-but-valid paths work.
        Self {
            steam_path: PathBuf::from(crate::steam::resolve::normalize_steam_path(&steam_path)),
        }
    }

    /// Validated installation root. Every mutating operation resolves through
    /// here first: a typo'd path fails with an actionable error instead of
    /// scattering `config/stplug-in` or `depotcache` folders at wrong locations.
    fn validated_root(&self) -> Result<PathBuf, String> {
        let raw = self.steam_path.to_string_lossy();
        crate::steam::resolve::resolve_steam_path(&raw)
            .map_err(|error| error.message(&raw))
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
        let _install_guard = manifest_install_gate()
            .lock()
            .map_err(|_| "Manifest installation scheduler is unavailable".to_string())?;
        let plugin_dir = self.validated_root()?.join("config").join("stplug-in");
        if !plugin_dir.exists() {
            fs::create_dir_all(&plugin_dir)
                .map_err(|e| format!("Failed to create plugin directory: {}", e))?;
        }

        let target_path = plugin_dir.join(format!("{}.lua", app_id));
        let temp_path = target_path.with_extension("tmp");

        // Prima di sovrascrivere, il .lua attuale viene preservato nell'albero
        // di backup AetherData (history/ se è una versione non ancora nota).
        // Niente più .lua.bak in stplug-in.
        if target_path.exists() {
            if let Ok(old_lua) = fs::read(&target_path) {
                if let Ok(backup) = crate::core::backup::GameBackup::for_app(app_id) {
                    let _ = backup.store_history_version(app_id, &old_lua);
                }
            }
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

        crate::desk_log_info!("steam", "Installed Lua config for AppID {} into {}", app_id, target_path.display());
        Ok(())
    }

    pub fn read_lua_config(&self, app_id: u32) -> Result<String, String> {
        let path = self
            .validated_root()?
            .join("config")
            .join("stplug-in")
            .join(format!("{}.lua", app_id));
        fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read plugin Lua {}: {}", path.display(), e))
    }

    /// Safely writes Steam depot .manifest files to Steam/depotcache.
    pub fn install_manifest_files(&self, manifests: &[ManifestPackageFile]) -> Result<usize, String> {
        if manifests.is_empty() {
            return Ok(0);
        }
        let _install_guard = manifest_install_gate()
            .lock()
            .map_err(|_| "Manifest installation scheduler is unavailable".to_string())?;

        let depotcache_dir = self.validated_root()?.join("depotcache");
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

            if manifest.bytes.is_empty() {
                return Err(format!("Refusing to install empty manifest {}", file_name));
            }
            let target_path = depotcache_dir.join(file_name);
            let temp_path = target_path.with_extension("tmp");
            fs::write(&temp_path, &manifest.bytes)
                .map_err(|e| format!("Failed to write temporary manifest {}: {}", file_name, e))?;
            fs::rename(&temp_path, &target_path)
                .map_err(|e| format!("Failed to install manifest {}: {}", file_name, e))?;
            let installed_len = fs::metadata(&target_path)
                .map_err(|e| format!("Failed to verify installed manifest {}: {}", file_name, e))?
                .len();
            if installed_len != manifest.bytes.len() as u64 {
                return Err(format!(
                    "Installed manifest {} is incomplete (expected {} bytes, found {})",
                    file_name,
                    manifest.bytes.len(),
                    installed_len
                ));
            }
            installed += 1;
        }

        crate::desk_log_info!("steam", "Installed {} manifest file(s) into Steam depotcache ({})", installed, depotcache_dir.display());
        Ok(installed)
    }

    /// Commits a complete Hubcap package as one guarded filesystem
    /// transaction. Every file is staged beside its final target, verified,
    /// and then atomically renamed; if any commit or post-commit check fails,
    /// the previous Lua/manifests are restored. This prevents a new Lua from
    /// ever being left pointing at a partially published manifest set.
    pub fn install_lua_and_manifest_files(
        &self,
        app_id: u32,
        lua_content: &str,
        manifests: &[ManifestPackageFile],
    ) -> Result<usize, String> {
        if lua_content.trim().is_empty() {
            return Err("Refusing to install an empty Lua package".to_string());
        }
        let _install_guard = manifest_install_gate()
            .lock()
            .map_err(|_| "Manifest installation scheduler is unavailable".to_string())?;
        let root = self.validated_root()?;
        let plugin_dir = root.join("config").join("stplug-in");
        let depotcache_dir = root.join("depotcache");
        fs::create_dir_all(&plugin_dir)
            .map_err(|error| format!("Failed to create plugin directory: {error}"))?;
        fs::create_dir_all(&depotcache_dir)
            .map_err(|error| format!("Failed to create depotcache directory: {error}"))?;

        // Normalize and validate the package before touching an existing file.
        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for manifest in manifests {
            let Some(name) = Path::new(&manifest.file_name)
                .file_name()
                .and_then(|value| value.to_str())
            else {
                return Err("Hubcap package contains an invalid manifest filename".to_string());
            };
            if !name.to_ascii_lowercase().ends_with(".manifest") {
                return Err(format!("Hubcap package contains a non-manifest file: {name}"));
            }
            if manifest.bytes.is_empty() {
                return Err(format!("Refusing to install empty manifest {name}"));
            }
            if let Some(previous) = files.insert(name.to_string(), manifest.bytes.clone()) {
                if previous != manifest.bytes {
                    return Err(format!("Hubcap package contains conflicting copies of {name}"));
                }
            }
        }

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let lua_target = plugin_dir.join(format!("{app_id}.lua"));
        let lua_temp = plugin_dir.join(format!(".{app_id}.{nonce}.lua.tmp"));
        fs::write(&lua_temp, lua_content)
            .map_err(|error| format!("Failed to stage plugin Lua: {error}"))?;

        let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
        for (index, (name, bytes)) in files.iter().enumerate() {
            let target = depotcache_dir.join(name);
            let temporary = depotcache_dir.join(format!(".{app_id}.{nonce}.{index}.manifest.tmp"));
            if let Err(error) = fs::write(&temporary, bytes) {
                let _ = fs::remove_file(&lua_temp);
                for (_, path) in &staged {
                    let _ = fs::remove_file(path);
                }
                return Err(format!("Failed to stage manifest {name}: {error}"));
            }
            staged.push((target, temporary));
        }

        let mut old_files: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::with_capacity(staged.len() + 1);
        old_files.push((lua_target.clone(), fs::read(&lua_target).ok()));
        for (target, _) in &staged {
            old_files.push((target.clone(), fs::read(target).ok()));
        }
        let mut committed: Vec<PathBuf> = Vec::new();
        macro_rules! rollback {
            ($error:expr) => {{
                for target in &committed {
                    let _ = fs::remove_file(target);
                }
                for (target, previous) in &old_files {
                    if let Some(bytes) = previous {
                        let _ = fs::write(target, bytes);
                    }
                }
                let _ = fs::remove_file(&lua_temp);
                for (_, temporary) in &staged {
                    let _ = fs::remove_file(temporary);
                }
                return Err($error);
            }};
        }

        // Move old targets out of the way first. This works on Windows too,
        // where rename-over-existing is not guaranteed by the standard API.
        if lua_target.exists() {
            if let Err(error) = fs::remove_file(&lua_target) {
                rollback!(format!("Could not replace existing {}: {error}", lua_target.display()));
            }
        }
        for (target, _) in &staged {
            if target.exists() {
                if let Err(error) = fs::remove_file(target) {
                    rollback!(format!("Could not replace existing {}: {error}", target.display()));
                }
            }
        }
        if let Err(error) = fs::rename(&lua_temp, &lua_target) {
            rollback!(format!("Failed to commit plugin Lua: {error}"));
        }
        committed.push(lua_target.clone());
        for (target, temporary) in &staged {
            if let Err(error) = fs::rename(temporary, target) {
                rollback!(format!("Failed to commit manifest {}: {error}", target.display()));
            }
            committed.push(target.clone());
        }

        let installed_lua = match fs::read_to_string(&lua_target) {
            Ok(content) => content,
            Err(error) => rollback!(format!("Failed to verify committed Lua: {error}")),
        };
        if installed_lua != lua_content {
            rollback!("Committed Lua differs from the downloaded package".to_string());
        }
        for (target, _) in &staged {
            let expected = files
                .get(target.file_name().and_then(|name| name.to_str()).unwrap_or_default())
                .map(Vec::len)
                .unwrap_or(0);
            let actual = fs::metadata(target).map(|metadata| metadata.len()).unwrap_or(0);
            if expected == 0 || actual != expected as u64 {
                rollback!(format!("Committed manifest {} failed verification", target.display()));
            }
        }

        if old_files[0].1.is_some() {
            if let Ok(backup) = crate::core::backup::GameBackup::for_app(app_id) {
                let _ = backup.store_history_version(app_id, &old_files[0].1.clone().unwrap_or_default());
            }
        }
        crate::desk_log_info!(
            "steam",
            "Atomically installed Hubcap package app_id={} lua=true manifests={}",
            app_id,
            staged.len()
        );
        Ok(staged.len())
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
