import { act, renderHook, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it } from 'vitest';
import type { StoreGameResult } from '../../hooks/useStoreSearch';
import { invokeMock, mockInvokeCommands } from '../../test/tauriMocks';
import { useTrendingStore } from './useTrendingStore';

const game = (id: number): StoreGameResult => ({
  id,
  appId: String(id),
  name: `Game ${id}`,
  has_manifest: true,
  has_denuvo: false,
});

const useHarness = () => {
  const [results, setResults] = useState<StoreGameResult[]>([]);
  return { results, trending: useTrendingStore(setResults) };
};

describe('useTrendingStore', () => {
  it('loads successive offsets and deduplicates game ids', async () => {
    invokeMock
      .mockResolvedValueOnce([game(1), game(2)])
      .mockResolvedValueOnce([game(2), game(3)]);
    const { result } = renderHook(useHarness);

    await act(() => result.current.trending.loadNext(2));
    await act(() => result.current.trending.loadNext(2));

    expect(invokeMock).toHaveBeenNthCalledWith(1, 'get_trending_store_games', { start: 0, count: 2 });
    expect(invokeMock).toHaveBeenNthCalledWith(2, 'get_trending_store_games', { start: 2, count: 2 });
    expect(result.current.results.map((item) => item.id)).toEqual([1, 2, 3]);
  });

  it('reserves unique offsets for concurrent top-ups', async () => {
    const resolvers: Array<(games: StoreGameResult[]) => void> = [];
    invokeMock.mockImplementation(() => new Promise((done) => { resolvers.push(done); }));
    const { result } = renderHook(useHarness);

    act(() => {
      void result.current.trending.loadNext(20);
      void result.current.trending.loadNext(20);
    });

    expect(invokeMock).toHaveBeenNthCalledWith(1, 'get_trending_store_games', { start: 0, count: 20 });
    expect(invokeMock).toHaveBeenNthCalledWith(2, 'get_trending_store_games', { start: 20, count: 20 });
    expect(result.current.trending.isLoading).toBe(true);
    await act(async () => {
      resolvers[0]?.([game(1)]);
      resolvers[1]?.([game(2)]);
    });
  });

  it('ignores a late response from a reset generation', async () => {
    let resolveOld: ((games: StoreGameResult[]) => void) | undefined;
    invokeMock.mockReturnValueOnce(new Promise((done) => { resolveOld = done; }));
    const { result } = renderHook(useHarness);

    act(() => { void result.current.trending.loadNext(20); });
    act(() => result.current.trending.reset());
    await act(async () => resolveOld?.([game(99)]));

    expect(result.current.results).toEqual([]);
    expect(result.current.trending.isLoading).toBe(false);
  });

  it('makes a failed offset retryable', async () => {
    invokeMock
      .mockRejectedValueOnce(new Error('network'))
      .mockResolvedValueOnce([game(1)]);
    const { result } = renderHook(useHarness);

    await act(() => result.current.trending.loadNext(20));
    await act(() => result.current.trending.loadNext(20));

    expect(invokeMock).toHaveBeenCalledTimes(2);
    await waitFor(() => expect(result.current.results).toEqual([game(1)]));
  });

  it('does not issue a backend request twice after a successful offset', async () => {
    mockInvokeCommands({ get_trending_store_games: [game(1)] });
    const { result } = renderHook(useHarness);

    await act(() => result.current.trending.loadNext(20));
    // The next public load uses the advanced offset rather than replaying zero.
    await act(() => result.current.trending.loadNext(20));

    expect(invokeMock).toHaveBeenNthCalledWith(2, 'get_trending_store_games', {
      start: 20,
      count: 20,
    });
  });
});
