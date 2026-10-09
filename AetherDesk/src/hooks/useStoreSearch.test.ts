import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { invokeMock } from '../test/tauriMocks';
import { useStoreSearch, type StoreGameResult } from './useStoreSearch';

const game = (id: number): StoreGameResult => ({
  id,
  appId: String(id),
  name: `Game ${id}`,
  has_manifest: true,
  has_denuvo: false,
});

describe('useStoreSearch', () => {
  it('serves a fresh cache hit without a network search', async () => {
    invokeMock.mockResolvedValueOnce({ cacheState: 'fresh', results: [game(1)] });
    const { result } = renderHook(useStoreSearch);

    await act(() => result.current.search('portal'));

    expect(result.current.results).toEqual([game(1)]);
    expect(result.current.activeQuery).toBe('portal');
    expect(result.current.hasSearched).toBe(true);
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('searches the backend after a cache miss', async () => {
    invokeMock
      .mockResolvedValueOnce({ cacheState: 'miss', results: [] })
      .mockResolvedValueOnce([game(2)]);
    const { result } = renderHook(useStoreSearch);

    await act(() => result.current.search('half-life'));

    expect(invokeMock).toHaveBeenNthCalledWith(2, 'search_store', { query: 'half-life' });
    expect(result.current.results).toEqual([game(2)]);
  });

  it('shows stale cache immediately and refreshes it in the background', async () => {
    let resolveRefresh: ((games: StoreGameResult[]) => void) | undefined;
    invokeMock
      .mockResolvedValueOnce({ cacheState: 'stale', results: [game(1)] })
      .mockReturnValueOnce(new Promise((resolve) => { resolveRefresh = resolve; }));
    const { result } = renderHook(useStoreSearch);

    await act(() => result.current.search('query'));
    expect(result.current.results).toEqual([game(1)]);
    expect(result.current.isLoading).toBe(false);

    await act(async () => resolveRefresh?.([game(2)]));
    expect(result.current.results).toEqual([game(2)]);
  });

  it('does not let an older search overwrite a newer one', async () => {
    let resolveOldCache: ((value: unknown) => void) | undefined;
    invokeMock
      .mockReturnValueOnce(new Promise((resolve) => { resolveOldCache = resolve; }))
      .mockResolvedValueOnce({ cacheState: 'fresh', results: [game(2)] });
    const { result } = renderHook(useStoreSearch);

    act(() => { void result.current.search('old'); });
    await act(() => result.current.search('new'));
    await act(async () => resolveOldCache?.({ cacheState: 'fresh', results: [game(1)] }));

    expect(result.current.activeQuery).toBe('new');
    expect(result.current.results).toEqual([game(2)]);
  });

  it('invalidates an in-flight request when cleared', async () => {
    let resolveCache: ((value: unknown) => void) | undefined;
    invokeMock.mockReturnValue(new Promise((resolve) => { resolveCache = resolve; }));
    const { result } = renderHook(useStoreSearch);

    act(() => { void result.current.search('query'); });
    act(() => result.current.clear());
    await act(async () => resolveCache?.({ cacheState: 'fresh', results: [game(1)] }));

    expect(result.current.results).toEqual([]);
    expect(result.current.hasSearched).toBe(false);
    expect(result.current.activeQuery).toBe('');
  });

  it('clears directly for whitespace-only input', async () => {
    const { result } = renderHook(useStoreSearch);

    await act(() => result.current.search('   '));

    expect(invokeMock).not.toHaveBeenCalled();
    await waitFor(() => expect(result.current.hasSearched).toBe(false));
  });
});
