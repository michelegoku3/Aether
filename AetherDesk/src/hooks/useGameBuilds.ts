import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

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
  const [builds, setBuilds] = useState<BuildInfo[]>([]);
  const [savedIds, setSavedIds] = useState<Set<number>>(new Set());
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const [all, saved] = await Promise.all([
        invoke<BuildInfo[]>('get_game_builds', { appId }),
        invoke<SavedBuild[]>('get_saved_builds', { appId }),
      ]);
      setBuilds(all || []);
      setSavedIds(new Set((saved || []).map((s) => s.buildId)));
    } catch (err: any) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, [appId]);

  useEffect(() => {
    load();
  }, [load]);

  const toggleSaved = useCallback(async (build: BuildInfo) => {
    const isSaved = savedIds.has(build.buildId);
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
  }, [appId, savedIds]);

  return { builds, savedIds, loading, error, reload: load, toggleSaved };
};
