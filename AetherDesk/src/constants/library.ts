/**
 * Tauri event names and payloads for Library/runtime pushes from the backend.
 * Keeping the public contract here prevents TSX consumers from hardcoding
 * strings and makes the Rust/WebView boundary explicit.
 */

export type LuaLibraryChangeOrigin =
  | 'store'
  | 'local'
  | 'library-action'
  | 'versioning'
  | 'filesystem'
  | 'settings';

/**
 * The backend emits this only when the observable Lua-library state has been
 * invalidated. It is deliberately not a partial game list: consumers must run
 * the same full scan as the Library Refresh button.
 */
export interface LuaLibraryChange {
  revision: number;
  origin: LuaLibraryChangeOrigin;
  scope: 'full-library';
  appIds: number[];
}

export const LUA_LIBRARY_EVENT = 'library://lua-changed';

/**
 * Emitted on every Steam runtime-state transition (running <-> stopped),
 * from `core/steam_monitor.rs`. Drives the Sidebar Start/Restart label.
 */
export const STEAM_RUNTIME_EVENT = 'steam://runtime-state';

/**
 * Emitted by `commands/versioning.rs` and `versioning/queue.rs` while a build
 * is being applied (step 10→95) and when background ACF edits finish (step
 * 100). Payload uses camelCase (serde rename_all = "camelCase").
 */
export const VERSIONING_PROGRESS_EVENT = 'versioning://progress';

export interface VersioningProgress {
  appId: number;
  buildId: number;
  step: number;   // 0..100
  message: string;
}

/**
 * Emitted by `core/hubcap_update_monitor.rs` only when the status snapshot
 * ACTUALLY changed (queue moved, a task completed/failed, a retry was
 * scheduled). Payload is the whole `MonitorStatus` snapshot, camelCase.
 *
 * Push-first, polling-as-recovery: the background-sync popup listens to this
 * and keeps a slow poll only so a missed event cannot leave it stale — the
 * same contract `library://lua-changed` + `get_library_change_revision` use.
 */
export const SYNC_STATUS_EVENT = 'sync://status-changed';
