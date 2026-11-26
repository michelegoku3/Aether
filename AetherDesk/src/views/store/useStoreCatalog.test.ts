import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { StoreGameResult } from '../../hooks/useStoreSearch';
import { useStoreCatalog } from './useStoreCatalog';

const { state, search, clear, setResults, trendingLoad, trendingReset, enrich, preload } = vi.hoisted(() => ({
  state: {
    results: [] as StoreGameResult[],
    isLoading: false,
    hasSearched: false,
    activeQuery: '',
  },
  search: vi.fn().mockResolvedValue(undefined),
  clear: vi.fn(),
  setResults: vi.fn(),
  trendingLoad: vi.fn().mockResolvedValue(undefined),
  trendingReset: vi.fn(),
  enrich: vi.fn().mockResolvedValue([]),
  preload: vi.fn(),
}));

vi.mock('../../hooks/useStoreSearch', () => ({
  useStoreSearch: () => ({ ...state, search, clear, setResults }),
}));
vi.mock('./useTrendingStore', () => ({
  useTrendingStore: () => ({ loadNext: trendingLoad, reset: trendingReset, isLoading: false }),
}));
vi.mock('../../hooks/useDenuvoEnrichment', () => ({ enrichDenuvoFlags: enrich }));
vi.mock('../../ui/GameCover', () => ({ preloadGameCovers: preload }));

const games = (count: number): StoreGameResult[] =>
  Array.from({ length: count }, (_, index) => ({
    id: index + 1,
    appId: String(index + 1),
    name: `Game ${index + 1}`,
    has_manifest: true,
    has_denuvo: false,
  }));

describe('useStoreCatalog', () => {
  beforeEach(() => {
    localStorage.clear();
    state.results = [];
    state.isLoading = false;
    state.hasSearched = false;
    state.activeQuery = '';
    for (const mock of [search, clear, setResults, trendingLoad, trendingReset, enrich, preload]) {
      mock.mockClear();
    }
  });

  it('paginates catalog results and tops up trending content', async () => {
    state.results = games(45);
    const { result } = renderHook(() => useStoreCatalog({ active: true, settingsReady: true, settingsRevision: 1 }));

    expect(result.current.pageGames).toHaveLength(20);
    expect(result.current.totalPages).toBe(3);
    act(() => result.current.setPage(2));
    expect(result.current.pageGames[0].appId).toBe('21');
    await waitFor(() => expect(trendingLoad).toHaveBeenCalledWith(40));
  });

  it('runs a query and resets pagination', async () => {
    const { result } = renderHook(() => useStoreCatalog({ active: true, settingsReady: false, settingsRevision: 0 }));
    act(() => result.current.setPage(3));

    await act(() => result.current.runSearch('portal'));

    expect(result.current.page).toBe(1);
    expect(search).toHaveBeenCalledWith('portal');
    expect(trendingReset).not.toHaveBeenCalled();
  });

  it('treats an empty query as clear and resets the trending session', async () => {
    const { result } = renderHook(() => useStoreCatalog({ active: true, settingsReady: false, settingsRevision: 0 }));

    await act(() => result.current.runSearch('   '));

    expect(clear).toHaveBeenCalledOnce();
    expect(trendingReset).toHaveBeenCalledOnce();
    expect(search).not.toHaveBeenCalled();
  });

  it('refreshes the active query once for each ready settings revision', async () => {
    state.activeQuery = 'portal';
    const { rerender } = renderHook(
      ({ revision }) => useStoreCatalog({ active: true, settingsReady: true, settingsRevision: revision }),
      { initialProps: { revision: 1 } }
    );
    await waitFor(() => expect(search).toHaveBeenCalledTimes(1));

    rerender({ revision: 1 });
    expect(search).toHaveBeenCalledTimes(1);
    rerender({ revision: 2 });
    await waitFor(() => expect(search).toHaveBeenCalledTimes(2));
    expect(trendingReset).toHaveBeenCalledTimes(2);
  });

  it('keeps the current page when the background interval refreshes', async () => {
    state.results = games(45);
    vi.useFakeTimers();
    try {
      const { result } = renderHook(() => useStoreCatalog({ active: true, settingsReady: true, settingsRevision: 1 }));
      act(() => result.current.setPage(3));
      expect(result.current.page).toBe(3);

      const clearCallsBefore = clear.mock.calls.length;
      const resetCallsBefore = trendingReset.mock.calls.length;
      await act(async () => {
        vi.advanceTimersByTime(3 * 60 * 1000);
      });

      expect(result.current.page).toBe(3);
      expect(clear.mock.calls.length).toBe(clearCallsBefore);
      expect(trendingReset.mock.calls.length).toBe(resetCallsBefore);
      expect(trendingLoad).toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps browse page membership in fetch order instead of re-sorting', () => {
    const older = { ...games(1)[0], id: 1, appId: '1', release_date_unix: 100 };
    const newer = { ...games(1)[0], id: 2, appId: '2', release_date_unix: 300 };
    state.results = [older, newer];
    const { result } = renderHook(() => useStoreCatalog({ active: true, settingsReady: true, settingsRevision: 1 }));

    // Sort is applied per loaded batch; re-sorting the accumulated catalog here
    // is what made a game leave page 1 after the next offset arrived.
    expect(result.current.pageGames.map((game) => game.id)).toEqual([1, 2]);
  });

  it('re-sorts the loaded browse catalog when the sort mode changes', () => {
    const zeta = { ...games(1)[0], id: 1, appId: '1', name: 'Zeta', release_date_unix: 300 };
    const alpha = { ...games(1)[0], id: 2, appId: '2', name: 'Alpha', release_date_unix: 100 };
    state.results = [zeta, alpha];
    const { result } = renderHook(() => useStoreCatalog({ active: true, settingsReady: true, settingsRevision: 1 }));

    act(() => result.current.setSortMode('name'));

    const sorted = setResults.mock.calls
      .map(([updater]) => updater as (items: StoreGameResult[]) => StoreGameResult[])
      .map((updater) => updater([zeta, alpha]))
      .find((items) => items.map((game) => game.id).join(',') === '2,1');
    expect(sorted).toBeDefined();
  });

  it('enriches only visible search results with Denuvo flags', async () => {
    state.results = games(2);
    state.activeQuery = 'query';
    enrich.mockResolvedValue([{ ...state.results[0], has_denuvo: true }, state.results[1]]);
    renderHook(() => useStoreCatalog({ active: true, settingsReady: false, settingsRevision: 0 }));

    await waitFor(() => expect(enrich).toHaveBeenCalledWith(state.results));
    expect(setResults).toHaveBeenCalledWith(expect.any(Function));
    const updater = setResults.mock.calls[0][0] as (items: StoreGameResult[]) => StoreGameResult[];
    expect(updater(state.results)[0].has_denuvo).toBe(true);
  });

  it('never opens a native alert when the search fails', async () => {
    search.mockRejectedValueOnce(new Error('provider timeout'));
    const alertSpy = vi.spyOn(window, 'alert').mockImplementation(() => {});
    const { result } = renderHook(() => useStoreCatalog({ active: true, settingsReady: false, settingsRevision: 0 }));

    await act(async () => {
      await result.current.runSearch('portal').catch(() => undefined);
    });

    expect(alertSpy).not.toHaveBeenCalled();
    alertSpy.mockRestore();
  });

  it('does not fetch or preload while the tab is hidden', async () => {
    state.results = games(25);
    const { rerender } = renderHook(
      ({ active }) => useStoreCatalog({ active, settingsReady: true, settingsRevision: 1 }),
      { initialProps: { active: false } }
    );

    await act(async () => {});
    expect(trendingLoad).not.toHaveBeenCalled();
    expect(preload).not.toHaveBeenCalled();

    rerender({ active: true });
    await waitFor(() => expect(trendingLoad).toHaveBeenCalledWith(40));
    expect(preload).toHaveBeenCalled();
  });

  it('defers Denuvo enrichment of a search until the tab is visible', async () => {
    state.results = games(2);
    state.activeQuery = 'query';
    enrich.mockResolvedValue(state.results);
    const { rerender } = renderHook(
      ({ active }) => useStoreCatalog({ active, settingsReady: true, settingsRevision: 1 }),
      { initialProps: { active: false } }
    );

    await act(async () => {});
    expect(enrich).not.toHaveBeenCalled();

    rerender({ active: true });
    await waitFor(() => expect(enrich).toHaveBeenCalled());
  });

  it('filters page games by the selected store badge filter', () => {
    const denuvoGame = { ...games(1)[0], id: 1, appId: '1', has_denuvo: true, drm_protection: 'Denuvo' };
    const uco2Game = { ...games(1)[0], id: 2, appId: '2', uco2_supported: true };
    const crackedGame = { ...games(1)[0], id: 3, appId: '3', crack_status: 'cracked' };
    const drmFreeGame = { ...games(1)[0], id: 4, appId: '4', drm_protection: 'None' };
    const regularGame = { ...games(1)[0], id: 5, appId: '5' };
    state.results = [denuvoGame, uco2Game, crackedGame, drmFreeGame, regularGame];

    const { result } = renderHook(() => useStoreCatalog({ active: true, settingsReady: true, settingsRevision: 1 }));

    expect(result.current.pageGames.map((g) => g.appId)).toEqual(['1', '2', '3', '4', '5']);

    act(() => result.current.setBadgeFilter('denuvo'));
    expect(result.current.pageGames.map((g) => g.appId)).toEqual(['1']);

    act(() => result.current.setBadgeFilter('uco2'));
    expect(result.current.pageGames.map((g) => g.appId)).toEqual(['2']);

    act(() => result.current.setBadgeFilter('cracked'));
    expect(result.current.pageGames.map((g) => g.appId)).toEqual(['3']);

    act(() => result.current.setBadgeFilter('drm_free'));
    expect(result.current.pageGames.map((g) => g.appId)).toEqual(['4']);

    act(() => result.current.setBadgeFilter('all'));
    expect(result.current.pageGames.map((g) => g.appId)).toEqual(['1', '2', '3', '4', '5']);
  });

  it('decorates games present in libraryGames and hides them when hideInLibrary is enabled', () => {
    state.results = games(3);
    const libraryGames = [
      {
        id: 2,
        name: 'Game 2',
        appId: '2',
        installDir: '',
        libraryPath: '',
        gamePath: '',
        installed: true,
        installing: false,
      },
    ];

    const { result } = renderHook(() =>
      useStoreCatalog({ active: true, settingsReady: true, settingsRevision: 1, libraryGames })
    );

    expect(result.current.hideInLibrary).toBe(false);
    expect(result.current.pageGames[1].in_library).toBe(true);
    expect(result.current.pageGames[1].installed).toBe(true);
    expect(result.current.pageGames[0].in_library).toBeUndefined();

    act(() => result.current.toggleHideInLibrary());
    expect(result.current.hideInLibrary).toBe(true);
    expect(localStorage.getItem('aether.store_hide_in_library')).toBe('true');
    expect(result.current.pageGames.map((g) => g.appId)).toEqual(['1', '3']);
  });

  it('syncs storeFrontFilter from defaultStoreFrontFilter and resets sort to relevance on search', async () => {
    state.results = games(2);
    const { result, rerender } = renderHook(
      ({ revision, front }) =>
        useStoreCatalog({
          active: true,
          settingsReady: true,
          settingsRevision: revision,
          defaultStoreFrontFilter: front,
        }),
      { initialProps: { revision: 1, front: 'top_sellers' } }
    );

    expect(result.current.storeFrontFilter).toBe('top_sellers');
    rerender({ revision: 2, front: 'discounts' });
    await waitFor(() => expect(result.current.storeFrontFilter).toBe('discounts'));

    act(() => result.current.setSortMode('newest'));
    expect(result.current.sortMode).toBe('newest');

    await act(() => result.current.runSearch('elden ring'));
    expect(result.current.sortMode).toBe('relevance');
  });

  it('keeps auto-loading trending batches when filters shrink filtered results below two pages', async () => {
    state.results = games(45);
    const allInLibrary = games(44).map((g) => ({
      id: g.id,
      name: g.name,
      appId: g.appId,
      installDir: '',
      libraryPath: '',
      gamePath: '',
      installed: true,
      installing: false,
    }));
    localStorage.setItem('aether.store_hide_in_library', 'true');

    renderHook(() =>
      useStoreCatalog({ active: true, settingsReady: true, settingsRevision: 1, libraryGames: allInLibrary })
    );

    await waitFor(() => expect(trendingLoad).toHaveBeenCalledWith(40));
  });
});
