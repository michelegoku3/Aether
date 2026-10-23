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
    error: null as string | null,
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
    state.results = [];
    state.isLoading = false;
    state.hasSearched = false;
    state.activeQuery = '';
    state.error = null;
    for (const mock of [search, clear, setResults, trendingLoad, trendingReset, enrich, preload]) {
      mock.mockClear();
    }
    // A queued rejection belongs to a single case: the next one starts from a
    // search that resolves.
    search.mockReset();
    search.mockResolvedValue(undefined);
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

  it('exposes the search error instead of alerting the user', async () => {
    const alertSpy = vi.spyOn(window, 'alert').mockImplementation(() => {});
    // The mocked search hook mirrors the real one: a failed search records the
    // message and drops the previous results.
    search.mockImplementationOnce(async () => {
      state.error = 'provider timeout';
      throw new Error('provider timeout');
    });
    const { result, rerender } = renderHook(() =>
      useStoreCatalog({ active: true, settingsReady: false, settingsRevision: 0 })
    );

    await act(async () => {
      await result.current.runSearch('portal').catch(() => undefined);
    });
    rerender();

    expect(result.current.searchError).toBe('provider timeout');
    expect(alertSpy).not.toHaveBeenCalled();
    alertSpy.mockRestore();
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
});
