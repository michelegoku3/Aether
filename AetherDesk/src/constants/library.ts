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
