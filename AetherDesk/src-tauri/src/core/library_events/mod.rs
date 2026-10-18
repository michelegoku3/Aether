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
//!
//! Module ownership:
//! - `snapshot`: everything the Library scan can see, as one comparable value
//!   (Lua fingerprints plus Steam's install projection) and the diff between two;
//! - `watch`: the single native watcher thread, its directory binding and the
//!   debounce/settle/rebind state machine;
//! - this file: the public contract (event name, payload, origin) and the
//!   controller commands call into.

mod snapshot;
mod watch;

use self::watch::run_watch_worker;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use tauri::{AppHandle, Emitter, Manager};

/// Stable Tauri contract consumed by the shared React Library store.
pub const LUA_LIBRARY_EVENT: &str = "library://lua-changed";

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
    pub(super) fn as_log_label(self) -> &'static str {
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

/// Commands the UI-facing API sends to the single watcher thread.
#[derive(Debug)]
pub(super) enum WatchCommand {
    Reconfigure(Option<PathBuf>),
    InternalChange {
        origin: LibraryChangeOrigin,
        app_ids: Vec<u32>,
    },
    Shutdown,
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

/// Where Aether's Lua lives inside a Steam installation. Shared layout
/// knowledge: `snapshot` reads it, `watch` must observe it.
pub(super) fn plugin_dir_for(steam_path: &Path) -> PathBuf {
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

pub(super) fn emit_change(
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
