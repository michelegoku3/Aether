/**
 * Tauri event names for library/data pushes from the backend.
 *
 * Kept in ONE place (single source of truth) so TSX consumers never hardcode
 * event strings. The Rust side emits these from `core/library_events.rs`
 * (library rescan requests) and `core/steam_monitor.rs` (runtime state).
 */

/**
 * Emitted by the backend the moment a `.lua` install operation SUCCEEDS
 * (store downloads from hubcap/luatool/ryuu/moed, local install/bulk
 * import, in-app removal). Consumers rescan the library in the background so
 * the change appears without a manual Refresh. Deliberately NOT filesystem-
 * driven: manual file operations outside AetherDesk use the Refresh button.
 */
export const LUA_LIBRARY_EVENT = 'library://lua-changed';

/**
 * Emitted on every Steam runtime-state transition (running <-> stopped),
 * from `core/steam_monitor.rs`. Drives the Sidebar Start/Restart label.
 */
export const STEAM_RUNTIME_EVENT = 'steam://runtime-state';
