import { useEffect, useRef, useState } from 'react';
import { enrichDenuvoFlags } from '../../hooks/useDenuvoEnrichment';
import { useStoreSearch } from '../../hooks/useStoreSearch';
import { preloadGameCovers } from '../../ui/GameCover';
import { useTrendingStore } from './useTrendingStore';

const ITEMS_PER_PAGE = 20;

interface Params {
  settingsRevision: number;
  settingsReady: boolean;
}

export function useStoreCatalog({ settingsRevision, settingsReady }: Params) {
  const [page, setPage] = useState(1);
  const observedSettingsRevision = useRef<number | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const storeSearch = useStoreSearch();
  const trending = useTrendingStore(storeSearch.setResults);
  const {
    results, setResults, isLoading, hasSearched, activeQuery, search, clear,
  } = storeSearch;

  useEffect(() => {
    if (!settingsReady || observedSettingsRevision.current === settingsRevision) return;
    observedSettingsRevision.current = settingsRevision;
    setPage(1);
    trending.reset();
    if (activeQuery.trim()) {
      search(activeQuery).catch((error) =>
        console.warn('Store search refresh after settings save failed:', error));
      return;
    }
    clear();
    // Refresh once per settings generation; search helpers intentionally keep
    // request ownership internally and need not retrigger this effect.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settingsRevision, settingsReady]);

  const totalPages = Math.ceil(results.length / ITEMS_PER_PAGE) || 1;
  const startIndex = (page - 1) * ITEMS_PER_PAGE;
  const pageGames = results.slice(startIndex, startIndex + ITEMS_PER_PAGE);
  const pageKey = pageGames.map((game) => game.appId).join(',');

  useEffect(() => {
    if (pageGames.length === 0 || document.visibilityState !== 'visible') return;
    preloadGameCovers(
      pageGames.map((game) => ({ appId: game.appId, imageUrl: game.imageUrl })),
      pageGames.length,
    );
    // pageKey represents the visible page identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pageKey]);

  useEffect(() => {
    if (!settingsReady || activeQuery.trim()) return;
    if (results.length < (page + 1) * ITEMS_PER_PAGE) {
      void trending.loadNext(ITEMS_PER_PAGE * 2);
    }
    // Session refs own offsets/in-flight requests; length/page are the top-up trigger.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page, results.length, activeQuery, settingsReady]);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
  }, [page, activeQuery]);

  useEffect(() => {
    if (pageGames.length === 0 || !activeQuery.trim()) return;
    let cancelled = false;
    enrichDenuvoFlags(pageGames)
      .then((enriched) => {
        if (cancelled) return;
        const flags = new Map(enriched.map((game) => [String(game.appId), game.has_denuvo]));
        setResults((current) => current.map((game) => {
          const flag = flags.get(String(game.appId));
          return flag === undefined ? game : { ...game, has_denuvo: flag };
        }));
      })
      .catch((error) => console.warn('Denuvo enrichment failed:', error));
    return () => { cancelled = true; };
    // pageKey is the complete enrichment request identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pageKey]);

  const runSearch = async (query: string) => {
    setPage(1);
    try {
      if (!query.trim()) {
        clear();
        trending.reset();
        return;
      }
      await search(query);
    } catch (error) {
      alert(`Search error: ${error}`);
    }
  };

  const clearSearch = () => {
    clear();
    trending.reset();
  };

  return {
    scrollRef,
    page,
    setPage,
    pageGames,
    totalPages,
    isLoading,
    isTrendingLoading: trending.isLoading,
    hasSearched,
    activeQuery,
    runSearch,
    clearSearch,
  };
}
