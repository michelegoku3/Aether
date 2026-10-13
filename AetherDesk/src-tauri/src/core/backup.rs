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
use crate::external_tools::fs::write_atomic;
use crate::manifest::package::ManifestPackageFile;
use crate::manifest::pins::LuaManifestPins;
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

    /// Private recovery metadata; the legacy text inventory stays unchanged.
    pub(crate) fn crack_recovery_path(&self, app_id: u32) -> PathBuf {
        self.root.join(format!("crack_{app_id}.state.json"))
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
        // A download from the sources is the PRISTINE version: it becomes/replaces
        // the original in the backup (the previous one goes to history/).
        self.store_original(app_id, lua_content.as_bytes())?;

        for manifest in manifest_files {
            let manifest_path = self.lua_dir().join(&manifest.file_name);
            write_atomic(&manifest_path, &manifest.bytes)?;
        }

        Ok(())
    }

    /// Copies every manifest referenced by the Lua from Steam/depotcache into
    /// this game's backup. Provider ZIPs already call `backup_lua_artifacts`,
    /// but a Lua imported from disk may reference manifests that only exist in
    /// Steam's cache; startup sync must capture those too.
    ///
    /// SHARED BACKUP CONTRACT — the AetherDLL twin is
    /// `BackupReferencedManifestsAtStartup` (hooks/wire/ManifestRestore.cpp),
    /// which can run while AetherDesk is closed. Both writers follow the same
    /// rules, so whichever process sees a manifest first produces the same
    /// backup tree:
    /// 1. only manifests referenced by the current Lua are copied;
    /// 2. a non-empty destination is never overwritten (the file name is the
    ///    manifest's complete depot/GID identity);
    /// 3. a zero-byte destination placeholder is removed and rewritten;
    /// 4. the copy is atomic (temp + rename) and size-verified;
    /// 5. a failed copy re-checks the destination: if the other writer
    ///    completed it in the race window, it counts as present, not failed.
    pub fn backup_referenced_manifests(
        &self,
        lua_content: &str,
        depotcache: &Path,
    ) -> ManifestBackupReport {
        let rows = LuaManifestPins::rows_from_content(lua_content);
        let mut report = ManifestBackupReport {
            scanned: rows.len(),
            ..ManifestBackupReport::default()
        };

        for row in rows {
            let file_name = format!("{}_{}.manifest", row.app_id, row.manifest_id);
            let Some(source) = find_case_insensitive_file(depotcache, &file_name) else {
                report.missing += 1;
                continue;
            };

            let destination = self.lua_dir().join(&file_name);
            if destination.is_file() {
                // A manifest filename contains its complete depot/GID identity;
                // retaining an existing non-empty copy is safer than replacing
                // it while Steam may still be writing the source file.
                if fs::metadata(&destination)
                    .map(|meta| meta.len() > 0)
                    .unwrap_or(false)
                {
                    report.unchanged += 1;
                    continue;
                }
                // A zero-byte placeholder is not a backup. Remove it so the
                // atomic writer also works on platforms where rename does not
                // replace an existing destination.
                let _ = fs::remove_file(&destination);
            }

            match fs::read(&source)
                .map_err(|error| format!("read {}: {}", source.display(), error))
                .and_then(|bytes| write_atomic(&destination, &bytes).map(|_| bytes))
            {
                Ok(bytes) => {
                    // Rule 4: the committed copy must match the bytes read.
                    let verified = fs::metadata(&destination)
                        .map(|meta| meta.len() == bytes.len() as u64)
                        .unwrap_or(false);
                    if verified {
                        report.copied += 1;
                    } else {
                        report.errors += 1;
                        crate::desk_log_debug!(
                            "backup",
                            "Backed-up manifest {} failed size verification",
                            source.display()
                        );
                    }
                }
                Err(error) => {
                    // Rule 5: benign race — AetherDLL's startup backup may
                    // have completed the same copy while we were working.
                    if fs::metadata(&destination)
                        .map(|meta| meta.len() > 0)
                        .unwrap_or(false)
                    {
                        report.unchanged += 1;
                    } else {
                        report.errors += 1;
                        crate::desk_log_debug!(
                            "backup",
                            "Could not back up manifest {}: {}",
                            source.display(),
                            error
                        );
                    }
                }
            }
        }

        report
    }

    /// Writes a single manifest whose bytes are already in memory (e.g. a
    /// freshly generated Workshop manifest) into this game's backup, following
    /// the shared backup contract: a non-empty destination is never
    /// overwritten (the file name is the manifest's complete depot/GID
    /// identity), a zero-byte placeholder is replaced, the write is atomic
    /// and size-verified. Returns true when a new copy was committed.
    pub fn backup_manifest_bytes(&self, file_name: &str, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return false;
        }
        let destination = self.lua_dir().join(file_name);
        if destination.is_file() {
            if fs::metadata(&destination)
                .map(|meta| meta.len() > 0)
                .unwrap_or(false)
            {
                return false;
            }
            let _ = fs::remove_file(&destination);
        }
        if write_atomic(&destination, bytes).is_err() {
            return false;
        }
        fs::metadata(&destination)
            .map(|meta| meta.len() == bytes.len() as u64)
            .unwrap_or(false)
    }
}

#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
pub struct ManifestBackupReport {
    pub scanned: usize,
    pub copied: usize,
    pub unchanged: usize,
    pub missing: usize,
    pub errors: usize,
}

fn find_case_insensitive_file(dir: &Path, file_name: &str) -> Option<PathBuf> {
    let exact = dir.join(file_name);
    if exact.is_file() {
        return Some(exact);
    }
    let entries = fs::read_dir(dir).ok()?;
    entries.flatten().map(|entry| entry.path()).find(|path| {
        path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case(file_name))
                .unwrap_or(false)
    })
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

// ============================================================================
// Startup sync: stplug-in -> backup/<app_id>/lua
// ============================================================================
// The lua backup is normally written by `backup_lua_artifacts` when a .lua
// flows through the app, but the user can also add or edit .lua files by hand
// in `config\stplug-in` (or change them with Change Version). This function,
// launched at startup in the background, brings the backup to the current state:
//
//   * .lua with no backup        -> backup created
//   * backup differing from .lua -> the previous version is archived in
//                                 `lua\history\<app_id>-<unix_secs>.lua` and the
//                                 backup updated (no "build" lost: this is the
//                                 base for the future "all versions" tab in
//                                 Change Version)
//   * identical backup           -> no write (byte comparison; .lua files
//                                 are small, reading is cheaper and more exact
//                                 than hash + disk signature)
//
// Files removed from stplug-in do NOT delete the backup: the last known
// version stays available (intended backup behavior).
// The function is pure blocking I/O: wrap it in spawn_blocking.

use sha2::{Digest, Sha256};

pub const LUA_HISTORY_SUBDIR: &str = "history";

/// Outcome of writing a lua version into the backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StoreLuaAction {
    Created,
    Updated,
    Unchanged,
}

/// Aggregate outcome of the sync (logs + future UI use).
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct LuaSyncReport {
    pub scanned: usize,
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub manifests_scanned: usize,
    pub manifests_copied: usize,
    pub manifests_unchanged: usize,
    pub manifests_missing: usize,
    pub manifest_errors: usize,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Millisecond-precision timestamp: two archives on the same day never
/// overwrite each other (bug of the seconds-precision version).
fn unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

impl GameBackup {
    /// Writes the ORIGINAL version of the .lua (the pristine one downloaded
    /// from the sources, or the first .lua seen in stplug-in when no backup
    /// exists).
    /// - identical -> Unchanged, no write
    /// - different  -> the previous original is archived in history/ and the
    ///   new one becomes the original (Updated)
    /// - missing    -> Created
    pub fn store_original(&self, app_id: u32, lua_bytes: &[u8]) -> Result<StoreLuaAction, String> {
        let lua_path = self.lua_dir().join(format!("{app_id}.lua"));
        match fs::read(&lua_path) {
            Ok(current) if current == lua_bytes => Ok(StoreLuaAction::Unchanged),
            Ok(old_bytes) => {
                if let Err(e) = self.archive_lua_history(app_id, &old_bytes) {
                    // L'archivio fallito non deve bloccare l'aggiornamento.
                    crate::desk_log_warn!(
                        "backup",
                        "Could not archive previous original Lua for {}: {}",
                        crate::core::logger::format_appid(app_id),
                        e
                    );
                }
                write_atomic(&lua_path, lua_bytes)?;
                let _ = write_atomic(
                    &self.lua_dir().join(format!("{app_id}.sha256")),
                    sha256_hex(lua_bytes).as_bytes(),
                );
                Ok(StoreLuaAction::Updated)
            }
            Err(_) => {
                write_atomic(&lua_path, lua_bytes)?;
                let _ = write_atomic(
                    &self.lua_dir().join(format!("{app_id}.sha256")),
                    sha256_hex(lua_bytes).as_bytes(),
                );
                Ok(StoreLuaAction::Created)
            }
        }
    }

    /// Archives a MODIFIED VERSION of the .lua in `lua\history\` (version
    /// change, manual edits, etc.). Deduplicated by content: when the version
    /// is already the original or already in history, writes nothing.
    /// Returns true when it wrote a new entry.
    pub fn store_history_version(&self, app_id: u32, lua_bytes: &[u8]) -> Result<bool, String> {
        if self.has_lua_version(app_id, lua_bytes) {
            return Ok(false);
        }
        self.archive_lua_history(app_id, lua_bytes).map(|_| true)
    }

    /// True when `lua_bytes` matches the original or a version already
    /// present in history (byte comparison, small files).
    pub fn has_lua_version(&self, app_id: u32, lua_bytes: &[u8]) -> bool {
        let original = self.lua_dir().join(format!("{app_id}.lua"));
        if fs::read(&original).is_ok_and(|current| current == lua_bytes) {
            return true;
        }
        let history_dir = self.lua_dir().join(LUA_HISTORY_SUBDIR);
        let Ok(entries) = fs::read_dir(&history_dir) else {
            return false;
        };
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("lua"))
            .any(|p| fs::read(&p).is_ok_and(|bytes| bytes == lua_bytes))
    }

    fn archive_lua_history(&self, app_id: u32, bytes: &[u8]) -> Result<std::path::PathBuf, String> {
        let history_dir = self.lua_dir().join(LUA_HISTORY_SUBDIR);
        fs::create_dir_all(&history_dir)
            .map_err(|e| format!("mkdir {}: {}", history_dir.display(), e))?;
        let millis = unix_millis();
        let mut path = history_dir.join(format!("{}-{}.lua", app_id, millis));
        // Same-millisecond archives (e.g. the pre+post Lua of one atomic pin
        // pipeline) must never overwrite each other: append a counter until
        // the name is unique. Chronological sort by name is preserved.
        let mut suffix = 1u32;
        while path.exists() {
            path = history_dir.join(format!("{}-{}-{}.lua", app_id, millis, suffix));
            suffix += 1;
        }
        fs::write(&path, bytes).map_err(|e| format!("write {}: {}", path.display(), e))?;
        crate::desk_log_info!(
            "backup",
            "Archived Lua version for {} -> {}",
            crate::core::logger::format_appid(app_id),
            path.display()
        );
        Ok(path)
    }
}

/// One entry of an app's archived lua version history.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LuaHistoryEntry {
    pub file_name: String,
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

/// Syncs lua backups by reading `steam_path\config\stplug-in`.
/// Blocking: call inside `spawn_blocking` (see main.rs).
pub fn sync_lua_backups_from_stplug_in(steam_path: &Path) -> LuaSyncReport {
    let mut report = LuaSyncReport::default();
    let plugin_dir = steam_path.join("config").join("stplug-in");
    let depotcache = steam_path.join("depotcache");

    let entries = match fs::read_dir(&plugin_dir) {
        Ok(entries) => entries,
        Err(error) => {
            crate::desk_log_warn!(
                "backup",
                "Lua backup sync skipped: cannot read {}: {}",
                plugin_dir.display(),
                error
            );
            return report;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("lua"))
            .unwrap_or(false)
        {
            continue; // skip .lua.bak and others
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(app_id) = stem.parse::<u32>() else {
            report.skipped += 1; // non-numeric name: not an app id
            continue;
        };
        report.scanned += 1;

        let Ok(src) = fs::read(&path) else {
            report.skipped += 1;
            continue;
        };

        // Model: the ORIGINAL is the pristine version (first .lua seen, or
        // the last one downloaded from the sources); any differing content
        // found in stplug-in is a modified version -> history/ (deduplicated
        // by content, so restarts create no copies).
        let backup = GameBackup::open_existing(app_id)
            .or_else(|| GameBackup::for_app(app_id).ok());
        let Some(backup) = backup else {
            report.skipped += 1;
            continue;
        };

        let original_path = backup.lua_dir().join(format!("{app_id}.lua"));
        if !original_path.exists() {
            match backup.store_original(app_id, &src) {
                Ok(_) => report.created += 1,
                Err(_) => report.skipped += 1,
            }
        } else {
            // Fast path: when the content matches the original there is
            // nothing to do — restarts generate no duplicate copies.
            let matches_original = fs::read(&original_path)
                .map(|original| original == src)
                .unwrap_or(false);
            if matches_original {
                report.unchanged += 1;
            } else {
                match backup.store_history_version(app_id, &src) {
                    Ok(true) => report.updated += 1,
                    Ok(false) => report.unchanged += 1,
                    Err(_) => report.skipped += 1,
                }
            }
        }

        // This pass is independent of the Lua version: even a known file may
        // have a manifest present in Steam/depotcache but never copied to
        // AetherData (e.g. local import or old installs).
        let manifest_report = backup.backup_referenced_manifests(
            std::str::from_utf8(&src).unwrap_or_default(),
            &depotcache,
        );
        report.manifests_scanned += manifest_report.scanned;
        report.manifests_copied += manifest_report.copied;
        report.manifests_unchanged += manifest_report.unchanged;
        report.manifests_missing += manifest_report.missing;
        report.manifest_errors += manifest_report.errors;
    }

    report
}

/// Lists the archived lua version history for an app (for the future "all
/// builds" tab in Change Version). Also includes the current backup as the
/// first entry when present.
pub fn list_lua_history(app_id: u32) -> Result<Vec<LuaHistoryEntry>, String> {
    let backup = GameBackup::for_app(app_id)?;
    let mut out = Vec::new();

    let current = backup.lua_dir().join(format!("{app_id}.lua"));
    if let Ok(bytes) = fs::read(&current) {
        out.push(LuaHistoryEntry {
            file_name: "original".to_string(),
            path: current.display().to_string(),
            size_bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        });
    }

    let history_dir = backup.lua_dir().join(LUA_HISTORY_SUBDIR);
    if history_dir.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(&history_dir)
            .map_err(|e| format!("Failed to read {}: {}", history_dir.display(), e))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("lua"))
            .collect();
        entries.sort();   // nome = <app_id>-<millis>: ordine cronologico
        for path in entries {
            if let Ok(bytes) = fs::read(&path) {
                out.push(LuaHistoryEntry {
                    file_name: path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?")
                        .to_string(),
                    path: path.display().to_string(),
                    size_bytes: bytes.len() as u64,
                    sha256: sha256_hex(&bytes),
                });
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
impl GameBackup {
    /// Test-only constructor: the production layout is anchored to
    /// `LocalAppPaths::data_root()`, which tests must not touch.
    pub(crate) fn for_tests(root: PathBuf) -> Self {
        Self { root }
    }
}

/// Contract tests for the SHARED backup rules documented in
/// `docs/shared_contracts.md` (the AetherDLL twin lives in
/// `hooks/wire/ManifestRestore.cpp`). If a rule changes here, the twin and
/// the spec must change together.
#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, GameBackup) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(LUA_SUBDIR)).expect("lua dir");
        let backup = GameBackup::for_tests(dir.path().to_path_buf());
        (dir, backup)
    }

    #[test]
    fn manifest_bytes_never_overwrites_non_empty_destination() {
        let (_dir, backup) = setup();
        assert!(backup.backup_manifest_bytes("1_2.manifest", b"first"));
        // Rule 2: same identity, different bytes -> the committed copy stays.
        assert!(!backup.backup_manifest_bytes("1_2.manifest", b"second-longer"));
        let committed = std::fs::read(backup.lua_dir().join("1_2.manifest")).unwrap();
        assert_eq!(committed, b"first");
    }

    #[test]
    fn manifest_bytes_replaces_zero_byte_placeholder() {
        let (_dir, backup) = setup();
        // Rule 3: a zero-byte destination is a placeholder, not a backup.
        std::fs::write(backup.lua_dir().join("3_4.manifest"), b"").unwrap();
        assert!(backup.backup_manifest_bytes("3_4.manifest", b"payload"));
        let committed = std::fs::read(backup.lua_dir().join("3_4.manifest")).unwrap();
        assert_eq!(committed, b"payload");
    }

    #[test]
    fn manifest_bytes_rejects_empty_payload() {
        let (_dir, backup) = setup();
        assert!(!backup.backup_manifest_bytes("5_6.manifest", b""));
        assert!(!backup.lua_dir().join("5_6.manifest").exists());
    }

    #[test]
    fn store_original_archives_previous_and_history_dedupes() {
        let (_dir, backup) = setup();
        assert_eq!(backup.store_original(42, b"v1").unwrap(), StoreLuaAction::Created);
        // Replacing the original archives the previous bytes in history/.
        assert_eq!(backup.store_original(42, b"v2").unwrap(), StoreLuaAction::Updated);
        assert_eq!(backup.store_original(42, b"v2").unwrap(), StoreLuaAction::Unchanged);
        assert!(backup.has_lua_version(42, b"v1"));
        // Dedup: v2 is already the original, archiving writes nothing.
        assert!(!backup.store_history_version(42, b"v2").unwrap());
        // A genuinely new modified version gets its own history entry.
        assert!(backup.store_history_version(42, b"v3-modified").unwrap());
        assert!(backup.has_lua_version(42, b"v3-modified"));
        // History entries never collide (millisecond names) and both exist.
        let history = std::fs::read_dir(backup.lua_dir().join(LUA_HISTORY_SUBDIR))
            .unwrap()
            .flatten()
            .count();
        assert_eq!(history, 2);
    }
}
