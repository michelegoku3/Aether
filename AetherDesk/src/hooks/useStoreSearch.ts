import { useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toErrorMessage } from '../util/errors';

export interface StoreGameResult {
  id: number;
  name: string;
  appId: string;
  has_manifest: boolean;
  has_denuvo: boolean;
  has_nsfw?: boolean;
  has_delisted?: boolean;
  imageUrl?: string;
  heroImageUrl?: string;
}

interface CachedStoreSearchResponse {
  results: StoreGameResult[];
  cacheState: 'fresh' | 'stale' | 'miss';
}

export const useStoreSearch = () => {
  const [results, setResults] = useState<StoreGameResult[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [hasSearched, setHasSearched] = useState(false);
  const [activeQuery, setActiveQuery] = useState('');
  const [error, setError] = useState<string | null>(null);
  const requestId = useRef(0);

  const clear = () => {
    requestId.current += 1;
    setResults([]);
    setHasSearched(false);
    setActiveQuery('');
    setError(null);
  };

  const applyResults = (query: string, games: StoreGameResult[]) => {
    // Denuvo enrichment is intentionally NOT done here: it runs per visible
    // page in StoreView, so a search costs at most ~20 rate-limited Steam
    // calls instead of one call per result.
    setResults(games || []);
    setActiveQuery(query);
  };

  const refreshInBackground = async (query: string, requestToken: number) => {
    try {
      const refreshedResults: StoreGameResult[] = await invoke('search_store', { query });
      if (requestId.current !== requestToken) return;
      applyResults(query, refreshedResults || []);
    } catch (err) {
      // Stale cache is already on screen; a refresh failure should not interrupt
      // the user. The next explicit search can try again.
      console.warn('[store] stale refresh failed:', err);
    }
  };

  const search = async (query: string) => {
    if (!query.trim()) {
      clear();
      return;
    }

    const currentRequest = requestId.current + 1;
    requestId.current = currentRequest;
    setIsLoading(true);
    setHasSearched(true);
    // A new attempt starts without the failure message of the previous one.
    setError(null);

    try {
      const cached: CachedStoreSearchResponse = await invoke('get_cached_store_search', { query });
      if (requestId.current !== currentRequest) return;

      if (cached.cacheState === 'fresh') {
        applyResults(query, cached.results || []);
        setIsLoading(false);
        return;
      }

      if (cached.cacheState === 'stale') {
        applyResults(query, cached.results || []);
        setIsLoading(false);
        refreshInBackground(query, currentRequest);
        return;
      }

      const initialResults: StoreGameResult[] = await invoke('search_store', { query });
      if (requestId.current !== currentRequest) return;
      applyResults(query, initialResults || []);
      setIsLoading(false);
    } catch (err) {
      if (requestId.current !== currentRequest) return;
      setIsLoading(false);
      // The failed query leaves nothing behind: the cards of the previous
      // query would contradict the failure reported for this one. The error
      // is set after the reset, which clears it.
      clear();
      setError(toErrorMessage(err));
      return;
    }
  };

  return {
    results,
    setResults,
    isLoading,
    hasSearched,
    activeQuery,
    error,
    search,
    clear,
  };
};
