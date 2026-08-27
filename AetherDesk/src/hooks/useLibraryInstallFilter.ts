import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getSettings } from './useSettings';
import type { InstalledGame } from './useLibraryGames';
import { filterAndSortGames } from '../util/search';

/** Persisted install-status filter for the Library toolbar. */
export type LibraryInstallFilter = 'all' | 'installed' | 'not_installed';

const FILTER_CYCLE: readonly LibraryInstallFilter[] = ['all', 'installed', 'not_installed'] as const;
const FILTER_PERSIST_DEBOUNCE_MS = 750;

type PersistWaiter = {
  resolve: () => void;
  reject: (reason: unknown) => void;
};

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
  searchQuery: string = '',
): string => {
  if (isLoading || !ready) return 'Scanning Lua library...';
  if (searchQuery.trim()) {
    return `${shown} games found`;
  }
  if (filter === 'all') {
    return `${total} games found`;
  }
  if (filter === 'installed') {
    return `${shown} installed`;
  }
  return `${shown} non-installed`;
};

const emptyMessageFor = (
  filter: LibraryInstallFilter,
  total: number,
  searchQuery: string = '',
): string => {
  if (total === 0) {
    return 'No installed Steam games were found. Check your Steam path in Settings and press Refresh.';
  }
  if (searchQuery.trim()) {
    return `No Lua games found matching "${searchQuery.trim()}".`;
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
 *
 * When `searchQuery` is provided, search searches across ALL games (ignoring
 * the active install filter) in real time.
 */
export const useLibraryInstallFilter = (
  games: InstalledGame[],
  isLoading: boolean,
  searchQuery: string = '',
) => {
  const [filter, setFilter] = useState<LibraryInstallFilter>('all');
  const [ready, setReady] = useState(false);
  const pendingPersistRef = useRef<{
    timer: number | undefined;
    latest: LibraryInstallFilter | undefined;
    waiters: PersistWaiter[];
  }>({ timer: undefined, latest: undefined, waiters: [] });

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

  const hasSearch = Boolean(searchQuery.trim());

  const filteredGames = useMemo(() => {
    // When searching, ignore the active filter and search all games in real time
    if (hasSearch) {
      return filterAndSortGames(games, searchQuery);
    }
    if (filter === 'installed') return games.filter((game) => game.installed);
    if (filter === 'not_installed') return games.filter((game) => !game.installed);
    return [...games].sort((a, b) => a.name.localeCompare(b.name));
  }, [games, filter, searchQuery, hasSearch]);

  const persist = useCallback((next: LibraryInstallFilter): Promise<void> => {
    return new Promise((resolve, reject) => {
      const pending = pendingPersistRef.current;
      pending.latest = next;
      pending.waiters.push({ resolve, reject });
      if (pending.timer !== undefined) {
        window.clearTimeout(pending.timer);
      }

      // Cycling the compact Library filter can happen several times in quick
      // succession. Persist only the final choice instead of atomically
      // rewriting settings.json (and the encrypted credential blob) per click.
      pending.timer = window.setTimeout(() => {
        const filterToPersist = pending.latest;
        const waiters = pending.waiters;
        pending.timer = undefined;
        pending.latest = undefined;
        pending.waiters = [];
        if (!filterToPersist) {
          waiters.forEach(({ resolve: done }) => done());
          return;
        }

        void (async () => {
          try {
            const settings = await getSettings();
            await invoke('save_settings', {
              settings: {
                ...settings,
                library_install_filter: filterToPersist,
              },
            });
            waiters.forEach(({ resolve: done }) => done());
          } catch (error) {
            waiters.forEach(({ reject: fail }) => fail(error));
          }
        })();
      }, FILTER_PERSIST_DEBOUNCE_MS);
    });
  }, []);

  useEffect(() => {
    return () => {
      const pending = pendingPersistRef.current;
      if (pending.timer !== undefined) {
        window.clearTimeout(pending.timer);
      }
      const cancelled = new Error('Library filter persistence was cancelled during shutdown.');
      pending.waiters.forEach(({ reject }) => reject(cancelled));
      pending.timer = undefined;
      pending.latest = undefined;
      pending.waiters = [];
    };
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
    countLabel: countLabelFor(filter, games.length, filteredGames.length, isLoading, ready, searchQuery),
    emptyMessage: emptyMessageFor(filter, games.length, searchQuery),
    /** CSS modifiers for the active filter button. */
    filterButtonClass:
      filter === 'installed'
        ? 'active filter-installed'
        : filter === 'not_installed'
          ? 'active filter-missing'
          : '',
  };
};
