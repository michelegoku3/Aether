import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useLibraryGames } from './useLibraryGames';
import { toErrorMessage } from '../util/errors';

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
 *
 * `toggleSaved` never rejects: a backend refusal keeps the bookmarks as they
 * were and is reported through `error`.
 */
export const useGameBuilds = (appId: number) => {
  // Both queries go through the shared per-game cache, so the Builds tab and
  // the actions popup share one in-flight request instead of duplicating it.
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
    } catch (err: unknown) {
      setError(toErrorMessage(err));
    } finally {
      setLoading(false);
    }
  }, [appId, queryGameState]);

  useEffect(() => {
    load();
  }, [load]);

  const toggleSaved = useCallback(
    async (build: BuildInfo) => {
      const isSaved = savedIds.has(build.buildId);
      // A new attempt starts from a clean slate: the message of the previous
      // failure must not stay on screen once the user tries again.
      setError('');
      try {
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
        // The bookmark list is cached with everything else: drop it now that
        // the backend accepted the change, so the next read sees the new file.
        invalidateGameState(appId);
      } catch (err: unknown) {
        // The backend refused the change: put the bookmark back the way it
        // was, so the star never claims a save the file does not hold. Only
        // the toggled build is restored, so a bookmark saved meanwhile stays.
        setSavedIds((prev) => {
          const next = new Set(prev);
          if (isSaved) next.add(build.buildId);
          else next.delete(build.buildId);
          return next;
        });
        setError(toErrorMessage(err));
      }
    },
    [appId, savedIds, invalidateGameState]
  );

  return { builds, savedIds, loading, error, reload: load, toggleSaved };
};
