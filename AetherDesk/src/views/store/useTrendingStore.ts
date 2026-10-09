import { useCallback, useRef, useState, type Dispatch, type SetStateAction } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { StoreGameResult } from '../../hooks/useStoreSearch';

interface TrendingSession {
  generation: number;
  requests: Set<number>;
  nextStart: number;
  inFlight: number;
}

const createSession = (generation: number): TrendingSession => ({
  generation,
  requests: new Set<number>(),
  nextStart: 0,
  inFlight: 0,
});

export function useTrendingStore(
  setResults: Dispatch<SetStateAction<StoreGameResult[]>>,
) {
  const [isLoading, setIsLoading] = useState(false);
  const sessionRef = useRef<TrendingSession>(createSession(0));

  const reset = useCallback(() => {
    sessionRef.current = createSession(sessionRef.current.generation + 1);
    setIsLoading(false);
  }, []);

  const mergeGames = useCallback((incoming: StoreGameResult[]) => {
    setResults((current) => {
      const seen = new Set(current.map((game) => Number(game.id)));
      const merged = [...current];
      for (const game of incoming) {
        if (!seen.has(Number(game.id))) {
          seen.add(Number(game.id));
          merged.push(game);
        }
      }
      return merged;
    });
  }, [setResults]);

  const load = useCallback(async (start: number, count: number) => {
    const session = sessionRef.current;
    const generation = session.generation;
    if (session.requests.has(start)) return;

    session.requests.add(start);
    session.nextStart = Math.max(session.nextStart, start + count);
    session.inFlight += 1;
    setIsLoading(true);
    try {
      const games = await invoke<StoreGameResult[]>('get_trending_store_games', { start, count });
      if (sessionRef.current.generation !== generation) return;
      mergeGames(games || []);
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
  }, [mergeGames]);

  const loadNext = useCallback((count: number) => {
    return load(sessionRef.current.nextStart, count);
  }, [load]);

  return { isLoading, reset, loadNext };
}
