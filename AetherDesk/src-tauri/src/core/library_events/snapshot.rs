//! Everything the Library scan can see, as one comparable value.
//!
//! Comparing two snapshots is the single question the watcher asks: "did
//! anything a user can observe change?". Nothing here parses or executes Lua,
//! and the install half keeps only the fields the Library exposes, so Steam's
//! per-second download churn cannot produce an invalidation.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use super::plugin_dir_for;
use crate::steam::library::{SteamInstallEntry, SteamLibraryScanner};

const MAX_SETTLE_RETRIES: u8 = 3;
pub(super) const SETTLE_RETRY_DELAY: Duration = Duration::from_millis(150);
const MAX_HASHED_LUA_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct LuaFingerprint {
    /// Metadata detects large files without reading unbounded content. For
    /// ordinary Lua files the digest below makes same-length edits reliable.
    bytes: u64,
    modified_ns: u128,
    digest: Option<[u8; 32]>,
}

/// Everything the Library scan can see, in one comparable value.
///
/// Both halves are invalidation inputs: `lua` is the game list, `install` is
/// Steam's own install state. Steam installs and uninstalls games without ever
/// touching stplug-in, so watching only the Lua half left the React Library
/// showing "not installed" until the user pressed Refresh.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct LibrarySnapshot {
    lua: BTreeMap<u32, LuaFingerprint>,
    install: BTreeMap<u32, SteamInstallEntry>,
}

pub(super) fn capture_snapshot_with_settle(steam_path: &Path) -> Result<LibrarySnapshot, String> {
    let mut last_error = None;
    for attempt in 0..=MAX_SETTLE_RETRIES {
        match capture_snapshot(steam_path) {
            Ok(snapshot) => return Ok(snapshot),
            Err(error) => {
                last_error = Some(error);
                if attempt < MAX_SETTLE_RETRIES {
                    thread::sleep(SETTLE_RETRY_DELAY);
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "unknown snapshot error".to_string()))
}

/// Both Library scan inputs in one comparable snapshot.
pub(super) fn capture_snapshot(steam_path: &Path) -> Result<LibrarySnapshot, String> {
    Ok(LibrarySnapshot {
        lua: capture_lua_snapshot(&plugin_dir_for(steam_path))?,
        // Cheap and churn-proof: `install_projection` keeps only the fields
        // the Library exposes, so Steam's download progress cannot invalidate.
        install: SteamLibraryScanner::new(steam_path).install_projection(),
    })
}

/// App IDs whose observable Library data differs between two snapshots.
///
/// Either input counts: a Lua edit, and an install, update or uninstall Steam
/// performed without touching stplug-in at all.
pub(super) fn changed_app_ids(before: &LibrarySnapshot, after: &LibrarySnapshot) -> Vec<u32> {
    before
        .lua
        .keys()
        .chain(after.lua.keys())
        .chain(before.install.keys())
        .chain(after.install.keys())
        .copied()
        .collect::<std::collections::BTreeSet<u32>>()
        .into_iter()
        .filter(|app_id| {
            before.lua.get(app_id) != after.lua.get(app_id)
                || before.install.get(app_id) != after.install.get(app_id)
        })
        .collect()
}

/// Effective Library membership and content signature without parsing or
/// executing Lua. Only canonical files that the scanner itself can display are
/// included, so unrelated temporary/backup files cannot refresh UI.
fn capture_lua_snapshot(plugin_dir: &Path) -> Result<BTreeMap<u32, LuaFingerprint>, String> {
    let entries = match fs::read_dir(plugin_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => {
            return Err(format!(
                "could not read {}: {}",
                plugin_dir.display(),
                error
            ));
        }
    };

    let mut files: Vec<(u32, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("could not read directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("could not inspect {}: {}", entry.path().display(), error))?;
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        if let Some(app_id) = canonical_lua_app_id(&path) {
            files.push((app_id, path));
        }
    }
    files.sort_by(|left, right| left.1.cmp(&right.1));

    let mut snapshot = BTreeMap::new();
    for (app_id, path) in files {
        // On normal Windows filesystems duplicate AppID names differing only
        // by case cannot exist. On a permissive filesystem choose the
        // lexicographically stable first path and log the anomaly.
        if snapshot.contains_key(&app_id) {
            crate::desk_log_warn!(
                "library-watch",
                "Ignoring duplicate canonical Lua entry for App ID {} at {}",
                app_id,
                path.display()
            );
            continue;
        }
        snapshot.insert(app_id, fingerprint_lua(&path)?);
    }
    Ok(snapshot)
}

fn canonical_lua_app_id(path: &Path) -> Option<u32> {
    let extension = path.extension()?.to_str()?;
    if !extension.eq_ignore_ascii_case("lua") {
        return None;
    }
    let app_id = path.file_stem()?.to_str()?.parse::<u32>().ok()?;
    (app_id > 0).then_some(app_id)
}

fn fingerprint_lua(path: &Path) -> Result<LuaFingerprint, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("could not inspect {}: {}", path.display(), error))?;
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);

    let digest = if metadata.len() <= MAX_HASHED_LUA_BYTES {
        let bytes = fs::read(path)
            .map_err(|error| format!("could not read {}: {}", path.display(), error))?;
        Some(Sha256::digest(bytes).into())
    } else {
        crate::desk_log_warn!(
            "library-watch",
            "Lua file {} exceeds {} bytes; using metadata-only change detection",
            path.display(),
            MAX_HASHED_LUA_BYTES
        );
        None
    };

    Ok(LuaFingerprint {
        bytes: metadata.len(),
        modified_ns,
        digest,
    })
}

#[cfg(test)]
mod tests {
    use super::{canonical_lua_app_id, capture_snapshot, changed_app_ids};
    use std::fs;
    use std::path::Path;

    /// Steam root with the two directories the watcher observes.
    pub(super) fn steam_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temporary directory");
        fs::create_dir_all(dir.path().join("config").join("stplug-in")).expect("create stplug-in");
        fs::create_dir_all(dir.path().join("steamapps")).expect("create steamapps");
        dir
    }

    fn lua_dir(root: &Path) -> std::path::PathBuf {
        root.join("config").join("stplug-in")
    }

    fn write_lua(root: &Path, app_id: u32, content: &str) {
        fs::write(lua_dir(root).join(format!("{app_id}.lua")), content).expect("write Lua");
    }

    fn acf_path(root: &Path, app_id: u32) -> std::path::PathBuf {
        root.join("steamapps")
            .join(format!("appmanifest_{app_id}.acf"))
    }

    /// Minimal but realistic `appmanifest`: the fields the scanner reads.
    fn write_acf(root: &Path, app_id: u32, state_flags: &str, bytes_downloaded: u64) {
        let body = format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"name\"\t\t\"Game {app_id}\"\n\t\"StateFlags\"\t\t\"{state_flags}\"\n\t\"installdir\"\t\t\"Game {app_id}\"\n\t\"BytesDownloaded\"\t\t\"{bytes_downloaded}\"\n}}\n"
        );
        fs::write(acf_path(root, app_id), body).expect("write ACF");
    }

    #[test]
    fn accepts_only_canonical_numeric_lua_names() {
        assert_eq!(canonical_lua_app_id(Path::new("123.lua")), Some(123));
        assert_eq!(canonical_lua_app_id(Path::new("123.LUA")), Some(123));
        assert_eq!(canonical_lua_app_id(Path::new("0.lua")), None);
        assert_eq!(canonical_lua_app_id(Path::new("123_build.lua")), None);
        assert_eq!(canonical_lua_app_id(Path::new("123.lua.bak")), None);
        assert_eq!(canonical_lua_app_id(Path::new("notes.txt")), None);
    }

    #[test]
    fn snapshot_detects_create_modify_and_remove_without_temp_noise() {
        let root = steam_root();
        let dir = lua_dir(root.path());
        fs::write(dir.join("100.lua"), "first").expect("write Lua");
        fs::write(dir.join("100.tmp"), "temporary").expect("write temp");
        fs::write(dir.join("100.lua.bak"), "backup").expect("write backup");

        let first = capture_snapshot(root.path()).expect("first snapshot");
        assert_eq!(first.lua.len(), 1);

        fs::write(dir.join("100.lua"), "second").expect("rewrite Lua");
        // Mirrors the writer/editor pattern: a non-Lua temporary file becomes
        // a canonical Library entry only once the atomic rename completes.
        fs::write(dir.join("200.tmp"), "new game").expect("write temporary Lua");
        fs::rename(dir.join("200.tmp"), dir.join("200.lua")).expect("atomic publish Lua");
        let second = capture_snapshot(root.path()).expect("second snapshot");
        assert_eq!(changed_app_ids(&first, &second), vec![100, 200]);

        fs::remove_file(dir.join("100.lua")).expect("remove Lua");
        let third = capture_snapshot(root.path()).expect("third snapshot");
        assert_eq!(changed_app_ids(&second, &third), vec![100]);
    }

    /// The second input of the Library scan: without it, installing or
    /// uninstalling a game in Steam never invalidated the React Library.
    #[test]
    fn install_state_changes_invalidate_the_library() {
        let root = steam_root();
        write_lua(root.path(), 100, "addappid(100, 1, \"key\")\n");

        let before = capture_snapshot(root.path()).expect("snapshot without ACF");
        assert!(!before.install.contains_key(&100));

        write_acf(root.path(), 100, "4", 0);
        let installed = capture_snapshot(root.path()).expect("snapshot with ACF");
        assert_eq!(
            installed.install.get(&100).map(|entry| entry.installed),
            Some(true)
        );
        assert_eq!(changed_app_ids(&before, &installed), vec![100]);

        fs::remove_file(acf_path(root.path(), 100)).expect("remove ACF");
        let gone = capture_snapshot(root.path()).expect("snapshot after uninstall");
        assert_eq!(changed_app_ids(&installed, &gone), vec![100]);
    }

    /// Steam rewrites the ACF continuously while downloading. The projection
    /// must ignore that churn, otherwise every progress write would trigger a
    /// full Library rescan every debounce window.
    #[test]
    fn download_progress_does_not_change_the_install_projection() {
        let root = steam_root();
        write_acf(root.path(), 100, "1026", 0);
        let started = capture_snapshot(root.path()).expect("queued snapshot");

        write_acf(root.path(), 100, "1026", 5_000_000);
        let downloaded = capture_snapshot(root.path()).expect("progress snapshot");
        assert_eq!(started.install, downloaded.install);
        assert!(changed_app_ids(&started, &downloaded).is_empty());

        // Steam finishing the install IS a change: the badge must flip.
        write_acf(root.path(), 100, "4", 5_000_000);
        let finished = capture_snapshot(root.path()).expect("finished snapshot");
        assert_eq!(changed_app_ids(&downloaded, &finished), vec![100]);
    }

    /// Cancelling/uninstalling an in-flight download leaves the ACF at Update
    /// Paused + Update Required. The content was never written, but the
    /// observable state changed from "Steam is writing" to "not writing": the
    /// Installing badge must go without a manual Refresh.
    #[test]
    fn cancelling_a_download_invalidates_the_library() {
        let root = steam_root();
        write_acf(root.path(), 100, "1026", 0);
        let downloading = capture_snapshot(root.path()).expect("downloading snapshot");

        write_acf(root.path(), 100, "530", 519_217_008);
        let paused = capture_snapshot(root.path()).expect("paused snapshot");

        assert_eq!(
            downloading
                .install
                .get(&100)
                .map(|entry| (entry.installed, entry.installing)),
            Some((false, true))
        );
        assert_eq!(
            paused
                .install
                .get(&100)
                .map(|entry| (entry.installed, entry.installing)),
            Some((false, false))
        );
        assert_eq!(changed_app_ids(&downloading, &paused), vec![100]);
    }
}
