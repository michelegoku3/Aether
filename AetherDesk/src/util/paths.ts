import { invoke } from '@tauri-apps/api/core';

/** Maximum length for a displayed path (80 chars, ellipsis dots included). */
export const MAX_PATH_CHARS = 80;

/**
 * Displays a path starting directly from the game root folder name, i.e. the
 * first folder right inside steamapps/common (e.g. "REPO\REPO.exe" instead of
 * "C:\Program Files (x86)\Steam\steamapps\common\REPO\REPO.exe").
 * Falls back to the absolute path when it is not under the game root.
 */
export const pathFromGameRoot = (gameRoot: string, absolutePath: string): string => {
  const root = gameRoot.replace(/[\\/]+$/, '');
  const rest = absolutePath.slice(root.length).replace(/^[\\/]+/, '');
  if (rest && absolutePath.toLowerCase().startsWith(root.toLowerCase())) {
    return rest;
  }
  return absolutePath;
};

/**
 * Shortens a path to at most `maxLength` characters total. When the path is
 * longer, it keeps `maxLength - 3` characters and appends "..." (the 3 dots
 * are included in the 80-character budget).
 */
export const shortenPath = (path: string, maxLength: number = MAX_PATH_CHARS): string => {
  if (path.length <= maxLength) return path;
  return `${path.slice(0, maxLength - 3)}...`;
};

/** Folder containing a file path (used to open it in the file manager). */
export const folderOf = (filePath: string): string => {
  const idx = Math.max(filePath.lastIndexOf('\\'), filePath.lastIndexOf('/'));
  return idx > 0 ? filePath.slice(0, idx) : filePath;
};

/**
 * Reusable helper: opens a folder in the system file manager.
 * Used by the Online panel (executable/DLL folders) and by the Settings view
 * (themes, wallpapers and icons folders).
 */
export const openInFileManager = async (folder: string): Promise<void> => {
  await invoke('reveal_in_file_manager', { path: folder });
};
