import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getSettings } from './useSettings';
import type { InstalledGame } from './useLibraryGames';

/** Persisted install-status filter for the Library toolbar. */
export type LibraryInstallFilter = 'all' | 'installed' | 'not_installed';

const FILTER_CYCLE: readonly LibraryInstallFilter[] = ['all', 'installed', 'not_installed'] as const;

export const normalizeLibraryInstallFilter = (value: unknown): LibraryInstallFilter => {
  const raw = String(value ?? '').trim().toLowerCase();
  if (raw === 'installed') return 'installed';
  if (raw === 'not_installed' || raw === 'not-installed' || raw === 'uninstalled') {
    return 'not_installed';
  }
  return 'all';
};

const filterTitleFor = (filter: LibraryInstallFilter): string => {
  switch (filter) {
    case 'installed':
      return 'Showing installed Lua games only — click to show non-installed';
    case 'not_installed':
      return 'Showing non-installed Lua games only — click to show all';
    default:
      return 'Showing all Lua games — click to show installed only';
  }
};

const countLabelFor = (
  filter: LibraryInstallFilter,
  total: number,
  shown: number,
  isLoading: boolean,
  ready: boolean,
): string => {
  if (isLoading || !ready) return 'Scanning Lua library...';
  if (filter === 'all') {
    return `${total} Lua game${total === 1 ? '' : 's'} found`;
  }
  if (filter === 'installed') {
    return `${shown} installed Lua game${shown === 1 ? '' : 's'} (${total} total)`;
  }
  return `${shown} non-installed Lua game${shown === 1 ? '' : 's'} (${total} total)`;
};

const emptyMessageFor = (filter: LibraryInstallFilter, total: number): string => {
  if (total === 0) {
    return 'No installed Steam games were found. Check your Steam path in Settings and press Refresh.';
  }
  if (filter === 'installed') {
    return 'No installed Lua games match this filter. Click the filter button or Refresh.';
  }
  if (filter === 'not_installed') {
    return 'No non-installed Lua games match this filter. Click the filter button or Refresh.';
  }
  return 'No installed Steam games were found. Check your Steam path in Settings and press Refresh.';
};

/**
 * Owns Library install-filter state: load/persist from settings.json, cycle
 * through modes, and derive the filtered list + UI copy. Keeps LibraryView
 * focused on layout and game actions (SRP / high cohesion).
 */
export const useLibraryInstallFilter = (games: InstalledGame[], isLoading: boolean) => {
  const [filter, setFilter] = useState<LibraryInstallFilter>('all');
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const settings = await getSettings();
        if (!cancelled) {
          setFilter(normalizeLibraryInstallFilter(settings.library_install_filter));
        }
      } catch {
        // Default "all" if settings cannot be read.
      } finally {
        if (!cancelled) setReady(true);
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, []);

  const filteredGames = useMemo(() => {
    if (filter === 'installed') return games.filter((game) => game.installed);
    if (filter === 'not_installed') return games.filter((game) => !game.installed);
    return games;
  }, [games, filter]);

  const persist = useCallback(async (next: LibraryInstallFilter) => {
    const settings = await getSettings();
    await invoke('save_settings', {
      settings: {
        ...settings,
        library_install_filter: next,
      },
    });
  }, []);

  const cycleFilter = useCallback(async () => {
    const currentIndex = FILTER_CYCLE.indexOf(filter);
    const next = FILTER_CYCLE[(currentIndex + 1) % FILTER_CYCLE.length];
    setFilter(next);
    await persist(next);
  }, [filter, persist]);

  return {
    filter,
    ready,
    filteredGames,
    cycleFilter,
    filterTitle: filterTitleFor(filter),
    countLabel: countLabelFor(filter, games.length, filteredGames.length, isLoading, ready),
    emptyMessage: emptyMessageFor(filter, games.length),
    /** CSS modifiers for the active filter button. */
    filterButtonClass:
      filter === 'installed'
        ? 'active filter-installed'
        : filter === 'not_installed'
          ? 'active filter-missing'
          : '',
  };
};
