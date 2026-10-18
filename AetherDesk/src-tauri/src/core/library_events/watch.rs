//! The single native watcher: directory binding plus the debounce/settle loop.
//!
//! The callback handed to `notify` only writes to a channel; every comparison,
//! read and Tauri emission happens here, on the worker thread. The loop also
//! owns rebinding: a new Steam library, stplug-in appearing or a refused
//! directory all move the set of directories that must be observed.

use notify::{recommended_watcher, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::AppHandle;

use super::snapshot::{
    capture_snapshot, capture_snapshot_with_settle, changed_app_ids, LibrarySnapshot,
    SETTLE_RETRY_DELAY,
};
use super::{emit_change, plugin_dir_for, LibraryChangeOrigin, WatchCommand};
use crate::steam::library::SteamLibraryScanner;

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(500);
const WORKER_TICK: Duration = Duration::from_millis(100);
const REBIND_DELAY: Duration = Duration::from_secs(2);

// The worker runs on a dedicated thread (`std::thread` requires `'static`): the
// by-value params ARE the ownership transfer into the thread; a reference would
// not compile at spawn. Clippy cannot see the consumption because `Receiver::recv`
// and the atomics take `&self`.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn run_watch_worker(
    app: AppHandle,
    command_rx: Receiver<WatchCommand>,
    revision: Arc<AtomicU64>,
) {
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

/// Creates the native watcher and binds it to every needed directory.
///
/// A directory that cannot be observed (a library on a disconnected drive)
/// is logged, recorded as pending and retried: it must not switch off
/// observation of the others. Only a watcher that could not be created, or
/// that observed nothing at all, is an error — the caller then retries after
/// `REBIND_DELAY`.
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

fn emit_next_change(
    app: &AppHandle,
    revision: &AtomicU64,
    origin: LibraryChangeOrigin,
    app_ids: Vec<u32>,
) {
    let next_revision = revision.fetch_add(1, Ordering::Relaxed) + 1;
    emit_change(app, next_revision, origin, app_ids);
}

#[cfg(test)]
mod tests {
    use super::bind_watcher;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn steam_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temporary directory");
        fs::create_dir_all(dir.path().join("config").join("stplug-in")).expect("create stplug-in");
        fs::create_dir_all(dir.path().join("steamapps")).expect("create steamapps");
        dir
    }

    fn lua_dir(root: &Path) -> PathBuf {
        root.join("config").join("stplug-in")
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
}
