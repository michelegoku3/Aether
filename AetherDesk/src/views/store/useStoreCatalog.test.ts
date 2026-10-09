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

const games = (count: number): StoreGameResult[] => Array.from({ length: count }, (_, index) => ({
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
    for (const mock of [search, clear, setResults, trendingLoad, trendingReset, enrich, preload]) {
      mock.mockClear();
    }
  });

  it('paginates catalog results and tops up trending content', async () => {
    state.results = games(45);
    const { result } = renderHook(() => useStoreCatalog({ settingsReady: true, settingsRevision: 1 }));

    expect(result.current.pageGames).toHaveLength(20);
    expect(result.current.totalPages).toBe(3);
    act(() => result.current.setPage(2));
    expect(result.current.pageGames[0].appId).toBe('21');
    await waitFor(() => expect(trendingLoad).toHaveBeenCalledWith(40));
  });

  it('runs a query and resets pagination', async () => {
    const { result } = renderHook(() => useStoreCatalog({ settingsReady: false, settingsRevision: 0 }));
    act(() => result.current.setPage(3));

    await act(() => result.current.runSearch('portal'));

    expect(result.current.page).toBe(1);
    expect(search).toHaveBeenCalledWith('portal');
    expect(trendingReset).not.toHaveBeenCalled();
  });

  it('treats an empty query as clear and resets the trending session', async () => {
    const { result } = renderHook(() => useStoreCatalog({ settingsReady: false, settingsRevision: 0 }));

    await act(() => result.current.runSearch('   '));

    expect(clear).toHaveBeenCalledOnce();
    expect(trendingReset).toHaveBeenCalledOnce();
    expect(search).not.toHaveBeenCalled();
  });

  it('refreshes the active query once for each ready settings revision', async () => {
    state.activeQuery = 'portal';
    const { rerender } = renderHook(
      ({ revision }) => useStoreCatalog({ settingsReady: true, settingsRevision: revision }),
      { initialProps: { revision: 1 } },
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
    renderHook(() => useStoreCatalog({ settingsReady: false, settingsRevision: 0 }));

    await waitFor(() => expect(enrich).toHaveBeenCalledWith(state.results));
    expect(setResults).toHaveBeenCalledWith(expect.any(Function));
    const updater = setResults.mock.calls[0][0] as (items: StoreGameResult[]) => StoreGameResult[];
    expect(updater(state.results)[0].has_denuvo).toBe(true);
  });
});
