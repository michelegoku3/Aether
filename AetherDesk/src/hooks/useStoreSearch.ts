import { useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface StoreGameResult {
  id: number;
  name: string;
  appId: string;
  has_manifest: boolean;
  has_denuvo: boolean;
  has_nsfw?: boolean;
  has_delisted?: boolean;
  imageUrl?: string;
}

export const useStoreSearch = () => {
  const [results, setResults] = useState<StoreGameResult[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [hasSearched, setHasSearched] = useState(false);
  const [activeQuery, setActiveQuery] = useState('');
  const requestId = useRef(0);

  const clear = () => {
    requestId.current += 1;
    setResults([]);
    setHasSearched(false);
    setActiveQuery('');
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

    try {
      const initialResults: StoreGameResult[] = await invoke('search_store', { query });
      if (requestId.current !== currentRequest) return;

      // Denuvo enrichment is intentionally NOT done here: it runs per visible
      // page in StoreView, so a search costs at most ~20 rate-limited Steam
      // calls instead of one call per result (which used to trip the limit).
      setResults(initialResults || []);
      setActiveQuery(query);
      setIsLoading(false);
    } catch (err) {
      if (requestId.current !== currentRequest) return;
      setIsLoading(false);
      throw err;
    }
  };

  return {
    results,
    setResults,
    isLoading,
    hasSearched,
    activeQuery,
    search,
    clear,
  };
};
