import { useCallback, useRef, useState, type Dispatch, type SetStateAction } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { compareStoreGames, type StoreSortMode } from '../../constants/store';
import type { StoreGameResult } from '../../hooks/useStoreSearch';

interface TrendingSession {
  generation: number;
  requests: Set<number>;
  nextStart: number;
  inFlight: number;
  exhausted: boolean;
}

const createSession = (generation: number): TrendingSession => ({
  generation,
  requests: new Set<number>(),
  nextStart: 0,
  inFlight: 0,
  exhausted: false,
});

export function useTrendingStore(
  setResults: Dispatch<SetStateAction<StoreGameResult[]>>,
  storeFrontFilter: string = 'trending',
  sortMode: StoreSortMode = 'trending'
) {
  const [isLoading, setIsLoading] = useState(false);
  const [hasMore, setHasMore] = useState(true);
  const sessionRef = useRef<TrendingSession>(createSession(0));
  const filterRef = useRef(storeFrontFilter);
  filterRef.current = storeFrontFilter;
  const sortRef = useRef(sortMode);
  sortRef.current = sortMode;

  const reset = useCallback(() => {
    sessionRef.current = createSession(sessionRef.current.generation + 1);
    setIsLoading(false);
    setHasMore(true);
  }, []);

  const mergeGames = useCallback(
    (incoming: StoreGameResult[]) => {
      // Each batch is ordered by itself and appended. Ordering the merged
      // catalog here would push games off the page the user is reading.
      const ordered = [...incoming].sort((a, b) => compareStoreGames(a, b, sortRef.current));
      setResults((current) => {
        const seen = new Set(current.map((game) => Number(game.id)));
        const merged = [...current];
        for (const game of ordered) {
          if (!seen.has(Number(game.id))) {
            seen.add(Number(game.id));
            merged.push(game);
          }
        }
        return merged;
      });
    },
    [setResults]
  );

  const load = useCallback(
    async (start: number, count: number) => {
      const session = sessionRef.current;
      const generation = session.generation;
      if (session.exhausted || session.requests.has(start)) return;

      session.requests.add(start);
      session.nextStart = Math.max(session.nextStart, start + count);
      session.inFlight += 1;
      setIsLoading(true);
      try {
        const games = await invoke<StoreGameResult[]>('get_trending_store_games', {
          start,
          count,
          storeFrontFilter: filterRef.current || null,
        });
        if (sessionRef.current.generation !== generation) return;
        const list = games || [];
        if (list.length === 0) {
          session.exhausted = true;
          setHasMore(false);
        }
        mergeGames(list);
      } catch (error) {
        if (sessionRef.current.generation === generation) {
          console.warn('Trending store preload failed:', error);
          session.requests.delete(start);
        }
      } finally {
        session.inFlight = Math.max(0, session.inFlight - 1);
        if (sessionRef.current.generation === generation && session.inFlight === 0) {
          setIsLoading(false);
        }
      }
    },
    [mergeGames]
  );

  const loadNext = useCallback(
    (count: number) => {
      return load(sessionRef.current.nextStart, count);
    },
    [load]
  );

  return { isLoading, hasMore, reset, loadNext };
}
