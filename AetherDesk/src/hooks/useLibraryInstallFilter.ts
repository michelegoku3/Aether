import { useCallback, useEffect, useMemo, useState } from 'react';
import type { InstalledGame } from './useLibraryGames';
import { filterAndSortGames } from '../util/search';

/** Persisted install-status filter for the Library toolbar. The active filter
 *  is UI-only state (it does not affect any backend behaviour), so it lives
 *  in localStorage instead of `save_settings`: rewriting settings.json — and
 *  with it the encrypted credential blob — on every filter click must be
 *  avoided. */
export type LibraryInstallFilter = 'all' | 'installed' | 'installing' | 'not_installed';

const FILTER_CYCLE: readonly LibraryInstallFilter[] = ['all', 'installed', 'installing', 'not_installed'] as const;
const LOCAL_FILTER_KEY = 'aether.library_install_filter';

const normalizeLibraryInstallFilter = (value: unknown): LibraryInstallFilter => {
  const raw = String(value ?? '')
    .trim()
    .toLowerCase();
  if (raw === 'installed') return 'installed';
  if (raw === 'installing' || raw === 'downloading' || raw === 'in-progress') {
    return 'installing';
  }
  if (raw === 'not_installed' || raw === 'not-installed' || raw === 'uninstalled') {
    return 'not_installed';
  }
  return 'all';
};

const readLocalFilter = (): LibraryInstallFilter | null => {
  try {
    const raw = localStorage.getItem(LOCAL_FILTER_KEY);
    if (raw == null) return null;
    return normalizeLibraryInstallFilter(raw);
  } catch {
    return null;
  }
};

const writeLocalFilter = (value: LibraryInstallFilter) => {
  try {
    localStorage.setItem(LOCAL_FILTER_KEY, value);
  } catch {
    /* ignore */
  }
};

const filterTitleFor = (filter: LibraryInstallFilter): string => {
  switch (filter) {
    case 'installed':
      return 'Click to show installing only';
    case 'installing':
      return 'Click to show non-installed only';
    case 'not_installed':
      return 'Click to show all';
    default:
      return 'Click to show installed only';
  }
};

const countLabelFor = (
  filter: LibraryInstallFilter,
  total: number,
  shown: number,
  isLoading: boolean,
  ready: boolean,
  searchQuery: string = ''
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
  if (filter === 'installing') {
    return `${shown} installing`;
  }
  return `${shown} non-installed`;
};

const emptyMessageFor = (filter: LibraryInstallFilter, total: number, searchQuery: string = ''): string => {
  if (total === 0) {
    return 'No installed Steam games were found. Check your Steam path in Settings and press Refresh.';
  }
  if (searchQuery.trim()) {
    return `No Lua games found matching "${searchQuery.trim()}".`;
  }
  if (filter === 'installed') {
    return 'No installed Lua games match this filter. Click the filter button or Refresh.';
  }
  if (filter === 'installing') {
    return 'No game is installing right now. Click the filter button or Refresh.';
  }
  if (filter === 'not_installed') {
    return 'No non-installed Lua games match this filter. Click the filter button or Refresh.';
  }
  return 'No installed Steam games were found. Check your Steam path in Settings and press Refresh.';
};

/**
 * Owns Library install-filter state: load/persist from localStorage, cycle
 * through modes, and derive the filtered list + UI copy. Keeps LibraryView
 * focused on layout and game actions (SRP / high cohesion).
 *
 * When `searchQuery` is provided, search searches across ALL games (ignoring
 * the active install filter) in real time.
 */
export const useLibraryInstallFilter = (games: InstalledGame[], isLoading: boolean, searchQuery: string = '') => {
  const [filter, setFilter] = useState<LibraryInstallFilter>('all');
  const [ready, setReady] = useState(false);

  useEffect(() => {
    setFilter(readLocalFilter() ?? 'all');
    setReady(true);
  }, []);

  const hasSearch = Boolean(searchQuery.trim());

  const filteredGames = useMemo(() => {
    // When searching, ignore the active filter and search all games in real time
    if (hasSearch) {
      return filterAndSortGames(games, searchQuery);
    }
    if (filter === 'installed') return games.filter((game) => game.installed);
    if (filter === 'installing') return games.filter((game) => game.installing);
    if (filter === 'not_installed') return games.filter((game) => !game.installed && !game.installing);
    return [...games].sort((a, b) => a.name.localeCompare(b.name));
  }, [games, filter, searchQuery, hasSearch]);

  /** Persists the new filter to localStorage synchronously and without any
   *  IPC — the preference is local UI state, not something the backend needs
   *  to know about. Returns a resolved Promise so callers that awaited the
   *  previous save_settings behaviour keep working. */
  const persist = useCallback((next: LibraryInstallFilter): Promise<void> => {
    writeLocalFilter(next);
    return Promise.resolve();
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
        : filter === 'installing'
          ? 'active filter-installing'
          : filter === 'not_installed'
            ? 'active filter-missing'
            : '',
  };
};
