import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useLibraryGames } from './useLibraryGames';

export interface BuildInfo {
  buildId: number;
  date: string;
  title: string;
}

export interface SavedBuild {
  appId: number;
  buildId: number;
  date: string;
  title: string;
  savedAt: number;
}

/**
 * Loads a game's build history and the user's saved-build bookmarks, and
 * exposes the save/unsave toggle. Single place where the versioning IPC is
 * called for the Builds tab.
 */
export const useGameBuilds = (appId: number) => {
  // Both queries are read through the shared per-game cache: the Builds tab
  // and the actions popup asked for the same data independently, and each
  // StrictMode mount doubled every request.
  const { queryGameState, invalidateGameState } = useLibraryGames();
  const [builds, setBuilds] = useState<BuildInfo[]>([]);
  const [savedIds, setSavedIds] = useState<Set<number>>(new Set());
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const [all, saved] = await Promise.all([
        queryGameState<BuildInfo[]>(appId, 'get_game_builds'),
        queryGameState<SavedBuild[]>(appId, 'get_saved_builds'),
      ]);
      setBuilds(all || []);
      setSavedIds(new Set((saved || []).map((s) => s.buildId)));
    } catch (err: any) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, [appId, queryGameState]);

  useEffect(() => {
    load();
  }, [load]);

  const toggleSaved = useCallback(async (build: BuildInfo) => {
    const isSaved = savedIds.has(build.buildId);
    // The bookmark list is cached with everything else: drop it as soon as the
    // user changes it, then keep the local state authoritative for this render.
    invalidateGameState(appId);
    if (isSaved) {
      await invoke('remove_saved_build', { appId, buildId: build.buildId });
      setSavedIds((prev) => {
        const next = new Set(prev);
        next.delete(build.buildId);
        return next;
      });
    } else {
      await invoke('save_build', {
        appId,
        buildId: build.buildId,
        date: build.date,
        title: build.title,
      });
      setSavedIds((prev) => new Set(prev).add(build.buildId));
    }
  }, [appId, savedIds, invalidateGameState]);

  return { builds, savedIds, loading, error, reload: load, toggleSaved };
};
