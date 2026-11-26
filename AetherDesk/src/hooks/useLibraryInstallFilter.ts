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
export type LibraryInstallFilter = 'all' | 'installed' | 'installing' | 'not_installed';

/** Badge filters use the flags the library already enriches, so switching one
 *  is pure client work: no rescan, no new IPC. */
export type LibraryBadgeFilter = 'all' | 'denuvo' | 'uco2' | 'cracked' | 'drm_free' | 'fix_pins';

export type LibrarySortMode = 'name' | 'installed_first' | 'issues_first';

export const LIBRARY_BADGE_FILTERS: { id: Exclude<LibraryBadgeFilter, 'fix_pins'>; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'denuvo', label: 'Denuvo' },
  { id: 'uco2', label: 'UCO2' },
  { id: 'cracked', label: 'Cracked' },
  { id: 'drm_free', label: 'DRM-Free' },
];

const LOCAL_BADGE_KEY = 'aether.library_badge_filter';
const LOCAL_SORT_KEY = 'aether.library_sort_mode';

const hasActiveLuaIssue = (game: InstalledGame): boolean =>
  (game.luaIssues ?? []).some((issue) => issue.active !== false);

const matchesBadgeFilter = (game: InstalledGame, badge: LibraryBadgeFilter): boolean => {
  const drm = (game.drm_protection ?? '').trim().toLowerCase();
  switch (badge) {
    case 'denuvo':
      return Boolean(game.has_denuvo) || drm.includes('denuvo');
    case 'uco2':
      return Boolean(game.uco2_supported);
    case 'cracked': {
      // isitcracked reports cracked / hypervisor variants; "uncracked" must
      // never match, so this is an allow-list rather than a substring test.
      const status = (game.crack_status ?? '').trim().toLowerCase();
      return status === 'cracked' || status.startsWith('hv') || status.includes('hypervisor');
    }
    case 'drm_free':
      return drm === 'none';
    case 'fix_pins':
      return hasActiveLuaIssue(game);
    default:
      return true;
  }
};

const readLocalBadgeFilter = (): LibraryBadgeFilter => {
  try {
    const raw = localStorage.getItem(LOCAL_BADGE_KEY);
    if (raw === 'fix_pins') return 'fix_pins';
    const match = LIBRARY_BADGE_FILTERS.find((entry) => entry.id === raw);
    return match ? match.id : 'all';
  } catch {
    return 'all';
  }
};

const readLocalSortMode = (): LibrarySortMode => {
  try {
    const raw = localStorage.getItem(LOCAL_SORT_KEY);
    if (raw === 'installed_first' || raw === 'issues_first' || raw === 'name') return raw;
    return 'name';
  } catch {
    return 'name';
  }
};

const installRank = (game: InstalledGame): number => {
  if (game.installed) return 0;
  if (game.installing) return 1;
  return 2;
};

const compareLibraryGames = (a: InstalledGame, b: InstalledGame, sortMode: LibrarySortMode): number => {
  if (sortMode === 'installed_first') {
    const diff = installRank(a) - installRank(b);
    if (diff !== 0) return diff;
  } else if (sortMode === 'issues_first') {
    const diff = Number(hasActiveLuaIssue(b)) - Number(hasActiveLuaIssue(a));
    if (diff !== 0) return diff;
  }
  return a.name.localeCompare(b.name);
};

const FILTER_CYCLE: readonly LibraryInstallFilter[] = ['all', 'installed', 'installing', 'not_installed'] as const;
const LOCAL_FILTER_KEY = 'aether.library_install_filter';
/** Legacy storage location: read once and then stop touching settings.json
 *  for this preference. */
const LEGACY_FILTER_KEY: keyof import('./useSettings').AppSettings = 'library_install_filter';

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
 * Owns Library install-filter state: load/persist from settings.json, cycle
 * through modes, and derive the filtered list + UI copy. Keeps LibraryView
 * focused on layout and game actions (SRP / high cohesion).
 *
 * When `searchQuery` is provided, search searches across ALL games (ignoring
 * the active install filter) in real time.
 */
export const useLibraryInstallFilter = (games: InstalledGame[], isLoading: boolean, searchQuery: string = '') => {
  const [filter, setFilter] = useState<LibraryInstallFilter>('all');
  const [badgeFilter, setBadgeFilterState] = useState<LibraryBadgeFilter>(readLocalBadgeFilter);
  const [sortMode, setSortModeState] = useState<LibrarySortMode>(readLocalSortMode);
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
    return () => {
      cancelled = true;
    };
  }, []);

  const hasSearch = Boolean(searchQuery.trim());

  const baseGames = useMemo(() => {
    // When searching, ignore the active install filter and search all games in
    // real time; the badge filter is orthogonal and still applies.
    if (hasSearch) {
      return filterAndSortGames(games, searchQuery);
    }
    const filtered =
      filter === 'installed'
        ? games.filter((game) => game.installed)
        : filter === 'installing'
          ? games.filter((game) => game.installing)
          : filter === 'not_installed'
            ? games.filter((game) => !game.installed && !game.installing)
            : [...games];
    return filtered.sort((a, b) => compareLibraryGames(a, b, sortMode));
  }, [games, filter, searchQuery, hasSearch, sortMode]);

  const badgeCounts = useMemo<Record<LibraryBadgeFilter, number>>(() => {
    const counts: Record<LibraryBadgeFilter, number> = {
      all: baseGames.length,
      denuvo: 0,
      uco2: 0,
      cracked: 0,
      drm_free: 0,
      fix_pins: 0,
    };
    for (const game of baseGames) {
      if (matchesBadgeFilter(game, 'denuvo')) counts.denuvo += 1;
      if (matchesBadgeFilter(game, 'uco2')) counts.uco2 += 1;
      if (matchesBadgeFilter(game, 'cracked')) counts.cracked += 1;
      if (matchesBadgeFilter(game, 'drm_free')) counts.drm_free += 1;
      if (matchesBadgeFilter(game, 'fix_pins')) counts.fix_pins += 1;
    }
    return counts;
  }, [baseGames]);

  const filteredGames = useMemo(
    () => (badgeFilter === 'all' ? baseGames : baseGames.filter((game) => matchesBadgeFilter(game, badgeFilter))),
    [baseGames, badgeFilter]
  );

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

  const setBadgeFilter = useCallback((next: LibraryBadgeFilter) => {
    setBadgeFilterState(next);
    try {
      localStorage.setItem(LOCAL_BADGE_KEY, next);
    } catch {
      /* localStorage can be unavailable; the in-memory filter still works. */
    }
  }, []);

  const setSortMode = useCallback((next: LibrarySortMode) => {
    setSortModeState(next);
    try {
      localStorage.setItem(LOCAL_SORT_KEY, next);
    } catch {
      /* localStorage can be unavailable; the in-memory sort still works. */
    }
  }, []);

  const countLabel =
    badgeFilter !== 'all'
      ? isLoading || !ready
        ? 'Scanning Lua library...'
        : `${filteredGames.length} ${filteredGames.length === 1 ? 'game' : 'games'} shown`
      : countLabelFor(filter, games.length, filteredGames.length, isLoading, ready, searchQuery);

  const badgeLabel =
    badgeFilter === 'fix_pins'
      ? 'Fix Pins'
      : (LIBRARY_BADGE_FILTERS.find((entry) => entry.id === badgeFilter)?.label ?? '');
  const emptyMessage =
    badgeFilter !== 'all' && !isLoading && ready
      ? `No games match the ${badgeLabel} filter.`
      : emptyMessageFor(filter, games.length, searchQuery);

  return {
    filter,
    ready,
    filteredGames,
    cycleFilter,
    badgeFilter,
    setBadgeFilter,
    badgeCounts,
    sortMode,
    setSortMode,
    filterTitle: filterTitleFor(filter),
    countLabel,
    emptyMessage,
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
