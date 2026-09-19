import { useCallback, useEffect, useMemo, useState } from 'react';
import { getSettings } from './useSettings';
import type { InstalledGame } from './useLibraryGames';
import { filterAndSortGames } from '../util/search';

/** Persisted install-status filter for the Library toolbar. The active filter
 *  is UI-only state (it does not affect any backend behaviour), so it lives
 *  in localStorage instead of `save_settings`. Rewriting settings.json — and
 *  with it the encrypted credential blob — on every filter click was a
 *  coupling bug flagged in the audit. On first run we migrate the legacy
 *  value from settings.json (if set) so users don't lose their preference. */
export type LibraryInstallFilter = 'all' | 'installed' | 'not_installed';

const FILTER_CYCLE: readonly LibraryInstallFilter[] = ['all', 'installed', 'not_installed'] as const;
const LOCAL_FILTER_KEY = 'aether.library_install_filter';
/** Legacy storage location: read once and then stop touching settings.json
 *  for this preference. */
const LEGACY_FILTER_KEY: keyof import('./useSettings').AppSettings = 'library_install_filter';

export const normalizeLibraryInstallFilter = (value: unknown): LibraryInstallFilter => {
  const raw = String(value ?? '').trim().toLowerCase();
  if (raw === 'installed') return 'installed';
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
  try { localStorage.setItem(LOCAL_FILTER_KEY, value); } catch { /* ignore */ }
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

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      // 1) localStorage wins (we migrated here on first run of the phase-1 fix).
      let initial: LibraryInstallFilter | null = readLocalFilter();
      // 2) First-run migration: pull the legacy value out of settings.json and
      //    re-home it in localStorage. After that, Settings saves no longer
      //    touch this field, so toggling the filter never triggers a file
      //    rewrite or re-encrypt of the credential blob.
      if (initial === null) {
        try {
          const settings = await getSettings();
          const legacy = normalizeLibraryInstallFilter(settings[LEGACY_FILTER_KEY]);
          initial = legacy;
          writeLocalFilter(legacy);
        } catch {
          initial = 'all';
        }
      }
      if (!cancelled) setFilter(initial ?? 'all');
      if (!cancelled) setReady(true);
    };
    void load();
    return () => { cancelled = true; };
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
        : filter === 'not_installed'
          ? 'active filter-missing'
          : '',
  };
};
