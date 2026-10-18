//! Lua library invalidation coordination.
//!
//! The Library never receives partial lists from the backend: it receives a single
//! invalidation and re-runs the same full scan as the Refresh button.
//! Invalidations come both from completed AetherDesk operations and from the
//! filesystem watcher, which observes the two inputs of that scan:
//! `<Steam>/config/stplug-in` (the game list) and the `steamapps` folder of every
//! discovered Steam library (Steam's own install state, which Steam changes
//! without ever touching stplug-in). Manual Lua edits and Steam-side installs,
//! updates and uninstalls therefore stay coherent with no WebView-side polling.

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

use crate::steam::library::{SteamInstallEntry, SteamLibraryScanner};

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

/// Everything the Library scan can see, in one comparable value.
///
/// Both halves are invalidation inputs: `lua` is the game list, `install` is
/// Steam's own install state. Steam installs and uninstalls games without ever
/// touching stplug-in, so watching only the Lua half left the React Library
/// showing "not installed" until the user pressed Refresh.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct LibrarySnapshot {
    lua: BTreeMap<u32, LuaFingerprint>,
    install: BTreeMap<u32, SteamInstallEntry>,
}

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
    /// Directories `notify` accepted.
    watched: Vec<PathBuf>,
    /// Directories the Steam root needs but `notify` refused. They are retried
    /// after `REBIND_DELAY` and must never be recorded as observed: a library
    /// that silently stops being watched is exactly the bug this watcher
    /// exists to prevent.
    unbound: Vec<PathBuf>,
}

impl WatchBinding {
    /// Every directory this binding accounts for, bound or pending retry.
    fn observed(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self.watched.iter().chain(&self.unbound).cloned().collect();
        paths.sort();
        paths
    }
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
            .send(WatchCommand::Reconfigure(normalize_steam_path(
                steam_path.as_ref(),
            )));
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

// The worker runs on a dedicated thread (`std::thread` requires `'static`): the
// by-value params ARE the ownership transfer into the thread; a reference would
// not compile at spawn. Clippy cannot see the consumption because `Receiver::recv`
// and the atomics take `&self`.
#[allow(clippy::needless_pass_by_value)]
fn run_watch_worker(app: AppHandle, command_rx: Receiver<WatchCommand>, revision: Arc<AtomicU64>) {
    let (fs_tx, fs_rx) = mpsc::channel::<notify::Result<Event>>();
    let mut desired_steam_path: Option<PathBuf> = None;
    let mut watcher: Option<RecommendedWatcher> = None;
    let mut binding: Option<WatchBinding> = None;
    let mut snapshot = LibrarySnapshot::default();
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
                snapshot = match desired_steam_path.as_deref() {
                    Some(steam_path) => match capture_snapshot(steam_path) {
                        Ok(snapshot) => snapshot,
                        Err(error) => {
                            crate::desk_log_warn!(
                                "library-watch",
                                "Could not establish Library watcher snapshot: {}",
                                error
                            );
                            LibrarySnapshot::default()
                        }
                    },
                    None => LibrarySnapshot::default(),
                };

                match desired_steam_path.as_deref() {
                    Some(steam_path) => match bind_library_watcher(steam_path, fs_tx.clone()) {
                        Ok((next_watcher, next_binding)) => {
                            adopt_binding(
                                "Library watcher",
                                next_watcher,
                                next_binding,
                                &mut watcher,
                                &mut binding,
                                &mut rebind_after,
                            );
                        }
                        Err(error) => {
                            crate::desk_log_warn!(
                                "library-watch",
                                "Library watcher is temporarily unavailable: {}. Retrying later.",
                                error
                            );
                            rebind_after = Some(Instant::now() + REBIND_DELAY);
                        }
                    },
                    None => {
                        crate::desk_log_info!(
                            "library-watch",
                            "Library watcher disabled because Steam path is not configured"
                        );
                    }
                }
            }
            Ok(WatchCommand::InternalChange { origin, app_ids }) => {
                let snapshot_result = desired_steam_path.as_deref().map(capture_snapshot);
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
                // An in-app write can also create stplug-in or add a Steam
                // library: keep the observed set aligned with the root.
                if desired_steam_path
                    .as_deref()
                    .is_some_and(|steam_path| watch_list_changed(steam_path, binding.as_ref()))
                {
                    watcher = None;
                    binding = None;
                    rebind_after = Some(Instant::now());
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
            rebind_after = desired_steam_path
                .as_ref()
                .map(|_| Instant::now() + REBIND_DELAY);
        }

        if rebind_after.is_some_and(|deadline| Instant::now() >= deadline) {
            rebind_after = None;
            if let Some(steam_path) = desired_steam_path.as_deref() {
                match bind_library_watcher(steam_path, fs_tx.clone()) {
                    Ok((next_watcher, next_binding)) => {
                        adopt_binding(
                            "Library watcher rebound",
                            next_watcher,
                            next_binding,
                            &mut watcher,
                            &mut binding,
                            &mut rebind_after,
                        );
                    }
                    Err(error) => {
                        crate::desk_log_warn!(
                            "library-watch",
                            "Library watcher rebind failed: {}. Retrying later.",
                            error
                        );
                        rebind_after = Some(Instant::now() + REBIND_DELAY);
                    }
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
        match capture_snapshot_with_settle(steam_path) {
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

                // A new Steam library, a removed one, or stplug-in appearing
                // changes which directories must be observed.
                if watch_list_changed(steam_path, binding.as_ref()) {
                    watcher = None;
                    binding = None;
                    rebind_after = Some(Instant::now());
                }
            }
            Err(error) => {
                crate::desk_log_warn!(
                    "library-watch",
                    "Could not settle Library changes under {}: {}",
                    steam_path.display(),
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

/// Every directory the watcher must observe for one Steam root.
///
/// The Lua plugin directory carries the game list; each discovered library's
/// `steamapps` carries the appmanifests Steam writes when it installs,
/// updates or uninstalls a game. Both are Library scan inputs, so both must be
/// able to produce an invalidation.
fn desired_watch_paths(steam_path: &Path) -> Vec<PathBuf> {
    let config_dir = steam_path.join("config");
    let plugin_dir = plugin_dir_for(steam_path);
    let mut paths = Vec::new();
    // Before stplug-in exists, `config` is watched so its creation is seen.
    if plugin_dir.is_dir() {
        paths.push(plugin_dir);
    } else if config_dir.is_dir() {
        paths.push(config_dir);
    }
    for library in SteamLibraryScanner::new(steam_path).discover_library_paths() {
        let steamapps = library.join("steamapps");
        if steamapps.is_dir() {
            paths.push(steamapps);
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

/// True when the bound directories no longer match what the root needs: a new
/// or removed Steam library, stplug-in appearing/disappearing, a new root.
/// A directory still waiting for its retry counts as accounted for, so the
/// retry timer — not every settle — drives the next attempt.
fn watch_list_changed(steam_path: &Path, binding: Option<&WatchBinding>) -> bool {
    match binding {
        None => true,
        Some(current) => {
            current.steam_path.as_path() != steam_path
                || current.observed() != desired_watch_paths(steam_path)
        }
    }
}

/// Creates the native watcher and binds it to every needed directory.
///
/// A directory that cannot be observed (a library on a disconnected drive)
/// is logged and skipped: it must not switch off observation of the others.
/// Only a watcher that could not be created, or that observed nothing at all,
/// is an error — the caller then retries after `REBIND_DELAY`.
/// Installs a successful bind into the worker state.
///
/// A directory `notify` refused stays in `binding.unbound` and arms the retry
/// timer, so the observation gap heals on its own instead of lasting until an
/// unrelated rebind.
fn adopt_binding(
    label: &str,
    watcher: RecommendedWatcher,
    binding: WatchBinding,
    watcher_slot: &mut Option<RecommendedWatcher>,
    binding_slot: &mut Option<WatchBinding>,
    rebind_after: &mut Option<Instant>,
) {
    crate::desk_log_info!(
        "library-watch",
        "{}: {} director{} watched{}",
        label,
        binding.watched.len(),
        if binding.watched.len() == 1 {
            "y"
        } else {
            "ies"
        },
        if binding.unbound.is_empty() {
            String::new()
        } else {
            format!(
                ", {} pending retry: {:?}",
                binding.unbound.len(),
                binding.unbound
            )
        }
    );
    if !binding.unbound.is_empty() {
        *rebind_after = Some(Instant::now() + REBIND_DELAY);
    }
    *watcher_slot = Some(watcher);
    *binding_slot = Some(binding);
}

/// Result of binding the native watcher: what `notify` accepted and what it
/// refused.
struct BoundWatcher {
    watcher: RecommendedWatcher,
    watched: Vec<PathBuf>,
    unbound: Vec<PathBuf>,
}

fn bind_watcher(
    paths: &[PathBuf],
    fs_tx: Sender<notify::Result<Event>>,
) -> Result<BoundWatcher, String> {
    if paths.is_empty() {
        return Err("no Steam directory to watch".to_string());
    }

    let mut watcher = recommended_watcher(move |event| {
        let _ = fs_tx.send(event);
    })
    .map_err(|error| format!("could not create native directory watcher: {error}"))?;

    let mut watched = Vec::new();
    let mut unbound = Vec::new();
    for path in paths {
        match watcher.watch(path, RecursiveMode::NonRecursive) {
            Ok(()) => {
                crate::desk_log_info!("library-watch", "Watching {}", path.display());
                watched.push(path.clone());
            }
            Err(error) => {
                crate::desk_log_warn!(
                    "library-watch",
                    "Could not watch {}: {} (retrying; the other directories stay observed)",
                    path.display(),
                    error
                );
                unbound.push(path.clone());
            }
        }
    }
    if watched.is_empty() {
        return Err(format!(
            "no directory could be watched ({} refused)",
            paths.len()
        ));
    }

    Ok(BoundWatcher {
        watcher,
        watched,
        unbound,
    })
}

fn bind_library_watcher(
    steam_path: &Path,
    fs_tx: Sender<notify::Result<Event>>,
) -> Result<(RecommendedWatcher, WatchBinding), String> {
    let paths = desired_watch_paths(steam_path);
    if paths.is_empty() {
        return Err(format!(
            "no Steam directory to watch under {}",
            steam_path.display()
        ));
    }

    let bound = bind_watcher(&paths, fs_tx)?;
    Ok((
        bound.watcher,
        WatchBinding {
            steam_path: steam_path.to_path_buf(),
            watched: bound.watched,
            unbound: bound.unbound,
        },
    ))
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

fn emit_change(app: &AppHandle, revision: u64, origin: LibraryChangeOrigin, app_ids: Vec<u32>) {
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

fn capture_snapshot_with_settle(steam_path: &Path) -> Result<LibrarySnapshot, String> {
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
fn capture_snapshot(steam_path: &Path) -> Result<LibrarySnapshot, String> {
    Ok(LibrarySnapshot {
        lua: capture_lua_snapshot(&plugin_dir_for(steam_path))?,
        // Cheap and churn-proof: `install_projection` keeps only the fields
        // the Library exposes, so Steam's download progress cannot invalidate.
        install: SteamLibraryScanner::new(steam_path).install_projection(),
    })
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

/// App IDs whose observable Library data differs between two snapshots.
///
/// Either input counts: a Lua edit, and an install, update or uninstall Steam
/// performed without touching stplug-in at all.
fn changed_app_ids(before: &LibrarySnapshot, after: &LibrarySnapshot) -> Vec<u32> {
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

#[cfg(test)]
mod tests {
    use super::{bind_watcher, canonical_lua_app_id, capture_snapshot, changed_app_ids};
    use std::fs;
    use std::path::Path;

    /// Steam root with the two directories the watcher observes.
    fn steam_root() -> tempfile::TempDir {
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

    /// A directory `notify` refuses must be reported as pending, never as
    /// observed: recording it as watched would make `watch_list_changed`
    /// permanently blind to it and that library would never be retried.
    #[test]
    fn a_directory_that_cannot_be_watched_is_reported_unbound() {
        let root = steam_root();
        let missing = root.path().join("not-a-directory");
        let paths = vec![lua_dir(root.path()), missing.clone()];
        let (fs_tx, _fs_rx) = std::sync::mpsc::channel();

        let bound = bind_watcher(&paths, fs_tx).expect("the first directory binds");

        assert_eq!(bound.watched, vec![lua_dir(root.path())]);
        assert_eq!(bound.unbound, vec![missing]);
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
}
