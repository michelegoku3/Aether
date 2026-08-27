//! Coordinamento delle invalidazioni della libreria Lua.
//!
//! La Library non riceve liste parziali dal backend: riceve una singola
//! invalidazione e riesegue la stessa scansione completa del pulsante Refresh.
//! Le invalidazioni provengono sia dalle operazioni concluse di AetherDesk sia
//! dal watcher della directory `<Steam>/config/stplug-in`, così le modifiche
//! manuali restano coerenti senza polling del filesystem lato WebView.

use notify::{recommended_watcher, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// Stable Tauri contract consumed by the shared React Library store.
pub const LUA_LIBRARY_EVENT: &str = "library://lua-changed";

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(500);
const WORKER_TICK: Duration = Duration::from_millis(100);
const REBIND_DELAY: Duration = Duration::from_secs(2);
const MAX_SETTLE_RETRIES: u8 = 3;
const SETTLE_RETRY_DELAY: Duration = Duration::from_millis(150);
const MAX_HASHED_LUA_BYTES: u64 = 8 * 1024 * 1024;

/// Why the observable Lua-library state changed. This is diagnostic context;
/// consumers always perform a full refresh and must not merge partial lists.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LibraryChangeOrigin {
    Store,
    Local,
    LibraryAction,
    Versioning,
    Filesystem,
    Settings,
}

impl LibraryChangeOrigin {
    fn as_log_label(self) -> &'static str {
        match self {
            Self::Store => "store",
            Self::Local => "local",
            Self::LibraryAction => "library-action",
            Self::Versioning => "versioning",
            Self::Filesystem => "filesystem",
            Self::Settings => "settings",
        }
    }
}

/// Intentionally small event payload. Paths, Lua text and provider secrets
/// never cross the Rust/WebView boundary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LuaLibraryChange {
    pub revision: u64,
    pub origin: LibraryChangeOrigin,
    pub scope: &'static str,
    pub app_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LuaFingerprint {
    /// Metadata detects large files without reading unbounded content. For
    /// ordinary Lua files the digest below makes same-length edits reliable.
    bytes: u64,
    modified_ns: u128,
    digest: Option<[u8; 32]>,
}

type LuaSnapshot = BTreeMap<u32, LuaFingerprint>;

#[derive(Debug)]
enum WatchCommand {
    Reconfigure(Option<PathBuf>),
    InternalChange {
        origin: LibraryChangeOrigin,
        app_ids: Vec<u32>,
    },
    Shutdown,
}

#[derive(Debug)]
struct WatchBinding {
    steam_path: PathBuf,
    plugin_dir: PathBuf,
    watching_plugin_dir: bool,
}

/// Single owner for filesystem observation. The callback supplied to `notify`
/// only writes to a channel; debouncing, I/O and Tauri event emission happen
/// in the worker, never on the OS watcher callback thread.
pub struct LibraryWatchController {
    command_tx: Sender<WatchCommand>,
    /// Exposed through a cheap Tauri command as a safety net for environments
    /// where a WebView loses a pushed event. It is not a filesystem poll: the
    /// watcher worker is the sole writer and this value is just an atomic read.
    revision: Arc<AtomicU64>,
}

impl LibraryWatchController {
    pub fn start(app: AppHandle, steam_path: impl AsRef<str>) -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let initial_path = normalize_steam_path(steam_path.as_ref());
        let revision = Arc::new(AtomicU64::new(0));
        let worker_revision = Arc::clone(&revision);

        thread::Builder::new()
            .name("aether-library-watch".to_string())
            .spawn(move || run_watch_worker(app, command_rx, worker_revision))
            .expect("failed to start Aether library watcher thread");

        // The first snapshot establishes the baseline. React performs its own
        // initial full scan, so startup never emits a spurious change event.
        let _ = command_tx.send(WatchCommand::Reconfigure(initial_path));
        Self {
            command_tx,
            revision,
        }
    }

    /// Rebind only when Settings changes the configured Steam installation.
    pub fn reconfigure(&self, steam_path: impl AsRef<str>) {
        let _ = self
            .command_tx
            .send(WatchCommand::Reconfigure(normalize_steam_path(steam_path.as_ref())));
    }

    /// Records a successful in-app mutation. The worker refreshes its disk
    /// snapshot before emitting so the later OS event from an atomic rename
    /// does not produce a second Library refresh.
    pub fn notify_internal_change(&self, origin: LibraryChangeOrigin, app_ids: Vec<u32>) {
        let _ = self
            .command_tx
            .send(WatchCommand::InternalChange { origin, app_ids });
    }

    /// Last invalidation revision observed by the worker. Reading it never
    /// touches Steam or the disk and is safe to call frequently from the UI.
    pub fn current_revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
}

impl Drop for LibraryWatchController {
    fn drop(&mut self) {
        let _ = self.command_tx.send(WatchCommand::Shutdown);
    }
}

/// Public façade used by commands after a Lua-affecting operation succeeds.
/// A direct emit fallback keeps the event contract available during unusual
/// startup/shutdown ordering, although normal application operation always
/// uses the managed coordinator.
pub fn notify_lua_changed(
    app: &AppHandle,
    origin: LibraryChangeOrigin,
    app_ids: impl IntoIterator<Item = u32>,
) {
    let app_ids = normalize_app_ids(app_ids);
    if let Some(controller) = app.try_state::<LibraryWatchController>() {
        controller.notify_internal_change(origin, app_ids);
        return;
    }

    emit_change(app, 0, origin, app_ids);
}

/// Reconfigure filesystem observation after a successfully persisted settings
/// change. It is deliberately a no-op if the controller is not yet managed.
pub fn reconfigure_library_watch(app: &AppHandle, steam_path: &str) {
    if let Some(controller) = app.try_state::<LibraryWatchController>() {
        controller.reconfigure(steam_path);
    }
}

/// Returns the native watcher revision for the frontend reconciliation safety
/// net. `0` means the controller has not been registered yet (startup/shutdown).
pub fn current_library_change_revision(app: &AppHandle) -> u64 {
    app.try_state::<LibraryWatchController>()
        .map(|controller| controller.current_revision())
        .unwrap_or(0)
}

fn run_watch_worker(
    app: AppHandle,
    command_rx: Receiver<WatchCommand>,
    revision: Arc<AtomicU64>,
) {
    let (fs_tx, fs_rx) = mpsc::channel::<notify::Result<Event>>();
    let mut desired_steam_path: Option<PathBuf> = None;
    let mut watcher: Option<RecommendedWatcher> = None;
    let mut binding: Option<WatchBinding> = None;
    let mut snapshot = LuaSnapshot::new();
    let mut dirty_since: Option<Instant> = None;
    let mut rebind_after: Option<Instant> = None;

    loop {
        match command_rx.recv_timeout(WORKER_TICK) {
            Ok(WatchCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                crate::desk_log_info!("library-watch", "Library watcher stopped");
                break;
            }
            Ok(WatchCommand::Reconfigure(next_path)) => {
                desired_steam_path = next_path;
                watcher = None;
                binding = None;
                dirty_since = None;
                rebind_after = None;
                snapshot = match desired_steam_path
                    .as_deref()
                    .map(plugin_dir_for)
                    .map(|path| capture_snapshot(&path))
                {
                    Some(Ok(snapshot)) => snapshot,
                    Some(Err(error)) => {
                        crate::desk_log_warn!(
                            "library-watch",
                            "Could not establish Library watcher snapshot: {}",
                            error
                        );
                        LuaSnapshot::new()
                    }
                    None => LuaSnapshot::new(),
                };

                match bind_watcher(desired_steam_path.as_deref(), fs_tx.clone()) {
                    Ok(Some((next_watcher, next_binding))) => {
                        crate::desk_log_info!(
                            "library-watch",
                            "Watching {} ({})",
                            next_binding.plugin_dir.display(),
                            if next_binding.watching_plugin_dir {
                                "stplug-in"
                            } else {
                                "config parent until stplug-in exists"
                            }
                        );
                        watcher = Some(next_watcher);
                        binding = Some(next_binding);
                    }
                    Ok(None) => {
                        crate::desk_log_info!(
                            "library-watch",
                            "Library watcher disabled because Steam path is not configured"
                        );
                    }
                    Err(error) => {
                        crate::desk_log_warn!(
                            "library-watch",
                            "Library watcher is temporarily unavailable: {}. Retrying later.",
                            error
                        );
                        rebind_after = desired_steam_path.as_ref().map(|_| Instant::now() + REBIND_DELAY);
                    }
                }
            }
            Ok(WatchCommand::InternalChange { origin, app_ids }) => {
                let snapshot_result = desired_steam_path
                    .as_deref()
                    .map(plugin_dir_for)
                    .map(|path| capture_snapshot(&path));
                // A Steam-path change must refresh even when the old and new
                // folders happen to have identical signatures. For ordinary
                // in-app writes, emit only if the watcher has not already
                // observed the same committed state.
                let mut should_emit = matches!(origin, LibraryChangeOrigin::Settings);
                match snapshot_result {
                    Some(Ok(next_snapshot)) => {
                        should_emit |= next_snapshot != snapshot;
                        snapshot = next_snapshot;
                    }
                    Some(Err(error)) => {
                        // The command already established that its Lua commit
                        // succeeded. Emit anyway; the next watcher batch can
                        // reconcile the snapshot when a transient read clears.
                        crate::desk_log_warn!(
                            "library-watch",
                            "Could not refresh watcher snapshot after {} change: {}",
                            origin.as_log_label(),
                            error
                        );
                        dirty_since = Some(Instant::now());
                        should_emit = true;
                    }
                    None => should_emit = true,
                }
                if should_emit {
                    emit_next_change(&app, revision.as_ref(), origin, app_ids);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        let mut watcher_reported_error = false;
        loop {
            match fs_rx.try_recv() {
                Ok(Ok(_event)) => {
                    // File filtering happens against a stable post-debounce
                    // snapshot. This also handles editors that rename a random
                    // temporary file over <appid>.lua.
                    dirty_since = Some(Instant::now());
                }
                Ok(Err(error)) => {
                    crate::desk_log_warn!(
                        "library-watch",
                        "Filesystem watcher reported an error: {}",
                        error
                    );
                    watcher_reported_error = true;
                    dirty_since = Some(Instant::now());
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }

        if watcher_reported_error {
            watcher = None;
            binding = None;
            rebind_after = desired_steam_path.as_ref().map(|_| Instant::now() + REBIND_DELAY);
        }

        if rebind_after.is_some_and(|deadline| Instant::now() >= deadline) {
            rebind_after = None;
            match bind_watcher(desired_steam_path.as_deref(), fs_tx.clone()) {
                Ok(Some((next_watcher, next_binding))) => {
                    crate::desk_log_info!(
                        "library-watch",
                        "Library watcher rebound to {}",
                        next_binding.plugin_dir.display()
                    );
                    watcher = Some(next_watcher);
                    binding = Some(next_binding);
                }
                Ok(None) => {}
                Err(error) => {
                    crate::desk_log_warn!(
                        "library-watch",
                        "Library watcher rebind failed: {}. Retrying later.",
                        error
                    );
                    rebind_after = desired_steam_path.as_ref().map(|_| Instant::now() + REBIND_DELAY);
                }
            }
        }

        let Some(first_signal) = dirty_since else {
            continue;
        };
        if first_signal.elapsed() < DEBOUNCE_WINDOW {
            continue;
        }
        dirty_since = None;

        let Some(steam_path) = desired_steam_path.as_deref() else {
            continue;
        };
        let plugin_dir = plugin_dir_for(steam_path);
        match capture_snapshot_with_settle(&plugin_dir) {
            Ok(next_snapshot) => {
                if next_snapshot != snapshot {
                    let changed_ids = changed_app_ids(&snapshot, &next_snapshot);
                    snapshot = next_snapshot;
                    emit_next_change(
                        &app,
                        revision.as_ref(),
                        LibraryChangeOrigin::Filesystem,
                        changed_ids,
                    );
                }

                // If stplug-in was removed, the previous native watch target is
                // no longer valid. Rebind to config so its recreation is seen.
                let must_rebind = binding
                    .as_ref()
                    .map(|current| {
                        current.steam_path.as_path() != steam_path
                            || (current.watching_plugin_dir != plugin_dir.is_dir())
                    })
                    .unwrap_or(true);
                if must_rebind {
                    watcher = None;
                    binding = None;
                    rebind_after = Some(Instant::now());
                }
            }
            Err(error) => {
                crate::desk_log_warn!(
                    "library-watch",
                    "Could not settle stplug-in changes at {}: {}",
                    plugin_dir.display(),
                    error
                );
                // A later event normally follows an editor save. The bounded
                // retry guarantees that a single atomic replacement also gets
                // one more chance without entering an infinite busy loop.
                dirty_since = Some(Instant::now() + SETTLE_RETRY_DELAY);
            }
        }
    }

    // Keep the watcher alive until the worker has fully stopped, then drop it
    // on this thread instead of from an OS callback.
    drop(watcher);
}

fn bind_watcher(
    steam_path: Option<&Path>,
    fs_tx: Sender<notify::Result<Event>>,
) -> Result<Option<(RecommendedWatcher, WatchBinding)>, String> {
    let Some(steam_path) = steam_path else {
        return Ok(None);
    };
    let config_dir = steam_path.join("config");
    let plugin_dir = config_dir.join("stplug-in");
    let (watch_path, watching_plugin_dir) = if plugin_dir.is_dir() {
        (plugin_dir.clone(), true)
    } else if config_dir.is_dir() {
        (config_dir, false)
    } else {
        return Err(format!("Steam config directory does not exist at {}", config_dir.display()));
    };

    let mut watcher = recommended_watcher(move |event| {
        let _ = fs_tx.send(event);
    })
    .map_err(|error| format!("could not create native directory watcher: {error}"))?;
    watcher
        .watch(&watch_path, RecursiveMode::NonRecursive)
        .map_err(|error| format!("could not watch {}: {error}", watch_path.display()))?;

    Ok(Some((
        watcher,
        WatchBinding {
            steam_path: steam_path.to_path_buf(),
            plugin_dir,
            watching_plugin_dir,
        },
    )))
}

fn plugin_dir_for(steam_path: &Path) -> PathBuf {
    steam_path.join("config").join("stplug-in")
}

fn normalize_steam_path(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

fn normalize_app_ids(app_ids: impl IntoIterator<Item = u32>) -> Vec<u32> {
    let mut ids: Vec<u32> = app_ids.into_iter().filter(|id| *id > 0).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn emit_next_change(
    app: &AppHandle,
    revision: &AtomicU64,
    origin: LibraryChangeOrigin,
    app_ids: Vec<u32>,
) {
    let next_revision = revision.fetch_add(1, Ordering::Relaxed) + 1;
    emit_change(app, next_revision, origin, app_ids);
}

fn emit_change(
    app: &AppHandle,
    revision: u64,
    origin: LibraryChangeOrigin,
    app_ids: Vec<u32>,
) {
    let payload = LuaLibraryChange {
        revision,
        origin,
        scope: "full-library",
        app_ids,
    };
    crate::desk_log_info!(
        "library-watch",
        "Library invalidated (revision={}, origin={}, app_ids={:?})",
        payload.revision,
        payload.origin.as_log_label(),
        payload.app_ids
    );
    if let Err(error) = app.emit(LUA_LIBRARY_EVENT, payload) {
        crate::desk_log_warn!(
            "library-watch",
            "Could not emit Library invalidation event: {}",
            error
        );
    }
}

fn capture_snapshot_with_settle(plugin_dir: &Path) -> Result<LuaSnapshot, String> {
    let mut last_error = None;
    for attempt in 0..=MAX_SETTLE_RETRIES {
        match capture_snapshot(plugin_dir) {
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

/// Returns the effective Library membership and content signature without
/// parsing or executing Lua. Only canonical files that the scanner itself can
/// display are included, so unrelated temporary/backup files cannot refresh UI.
fn capture_snapshot(plugin_dir: &Path) -> Result<LuaSnapshot, String> {
    let entries = match fs::read_dir(plugin_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(LuaSnapshot::new()),
        Err(error) => {
            return Err(format!("could not read {}: {}", plugin_dir.display(), error));
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

    let mut snapshot = LuaSnapshot::new();
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

fn changed_app_ids(before: &LuaSnapshot, after: &LuaSnapshot) -> Vec<u32> {
    before
        .keys()
        .chain(after.keys())
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|app_id| before.get(app_id) != after.get(app_id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{capture_snapshot, canonical_lua_app_id, changed_app_ids};
    use std::fs;
    use std::path::Path;

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
        let temp = tempfile::tempdir().expect("temporary directory");
        let dir = temp.path();
        fs::write(dir.join("100.lua"), "first").expect("write Lua");
        fs::write(dir.join("100.tmp"), "temporary").expect("write temp");
        fs::write(dir.join("100.lua.bak"), "backup").expect("write backup");

        let first = capture_snapshot(dir).expect("first snapshot");
        assert_eq!(first.len(), 1);

        fs::write(dir.join("100.lua"), "second").expect("rewrite Lua");
        // Mirrors the writer/editor pattern: a non-Lua temporary file becomes
        // a canonical Library entry only once the atomic rename completes.
        fs::write(dir.join("200.tmp"), "new game").expect("write temporary Lua");
        fs::rename(dir.join("200.tmp"), dir.join("200.lua")).expect("atomic publish Lua");
        let second = capture_snapshot(dir).expect("second snapshot");
        assert_eq!(changed_app_ids(&first, &second), vec![100, 200]);

        fs::remove_file(dir.join("100.lua")).expect("remove Lua");
        let third = capture_snapshot(dir).expect("third snapshot");
        assert_eq!(changed_app_ids(&second, &third), vec![100]);
    }
}
