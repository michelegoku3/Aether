import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { CACHE_CLEARED_EVENT } from '../../constants/library';
import { enrichDenuvoFlags } from '../../hooks/useDenuvoEnrichment';
import { useStoreSearch } from '../../hooks/useStoreSearch';
import { preloadGameCovers } from '../../ui/GameCover';
import { useTrendingStore } from './useTrendingStore';

const ITEMS_PER_PAGE = 20;

interface Params {
  /** Hidden views stay mounted; only the visible tab may start network work. */
  active: boolean;
  settingsRevision: number;
  settingsReady: boolean;
}

export function useStoreCatalog({ active, settingsRevision, settingsReady }: Params) {
  const [page, setPage] = useState(1);
  const observedSettingsRevision = useRef<number | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const storeSearch = useStoreSearch();
  const trending = useTrendingStore(storeSearch.setResults);
  const { results, setResults, isLoading, hasSearched, activeQuery, error, search, clear } = storeSearch;
  const clearRef = useRef(clear);
  clearRef.current = clear;
  const trendingRef = useRef(trending);
  trendingRef.current = trending;

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen(CACHE_CLEARED_EVENT, () => {
      // "Clear caches" invalidated the backend's store cache. Drop what is on
      // screen; the active/query effect above decides when the next trending
      // preload is allowed to start, so a hidden Store tab does no network work.
      clearRef.current();
      trendingRef.current.reset();
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((error) => {
        console.warn('[store] cache-clear bridge unavailable:', error);
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!active || !settingsReady || observedSettingsRevision.current === settingsRevision) return;
    observedSettingsRevision.current = settingsRevision;
    setPage(1);
    trending.reset();
    if (activeQuery.trim()) {
      // A failed refresh surfaces through the hook's error state.
      void search(activeQuery);
      return;
    }
    clear();
    // Refresh once per settings generation; search helpers intentionally keep
    // request ownership internally and need not retrigger this effect.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, settingsRevision, settingsReady]);

  const totalPages = Math.ceil(results.length / ITEMS_PER_PAGE) || 1;
  const startIndex = (page - 1) * ITEMS_PER_PAGE;
  const pageGames = results.slice(startIndex, startIndex + ITEMS_PER_PAGE);
  const pageKey = pageGames.map((game) => game.appId).join(',');

  useEffect(() => {
    if (!active || pageGames.length === 0 || document.visibilityState !== 'visible') return;
    preloadGameCovers(
      pageGames.map((game) => ({ appId: game.appId, imageUrl: game.imageUrl })),
      pageGames.length
    );
    // pageKey represents the visible page identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, pageKey]);

  useEffect(() => {
    if (!active || !settingsReady || activeQuery.trim()) return;
    if (results.length < (page + 1) * ITEMS_PER_PAGE) {
      void trending.loadNext(ITEMS_PER_PAGE * 2);
    }
    // Session refs own offsets/in-flight requests; length/page are the top-up trigger.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, page, results.length, activeQuery, settingsReady]);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
  }, [page, activeQuery]);

  useEffect(() => {
    if (!active || pageGames.length === 0 || !activeQuery.trim()) return;
    let cancelled = false;
    enrichDenuvoFlags(pageGames)
      .then((enriched) => {
        if (cancelled) return;
        const flags = new Map(enriched.map((game) => [String(game.appId), game.has_denuvo]));
        setResults((current) =>
          current.map((game) => {
            const flag = flags.get(String(game.appId));
            return flag === undefined ? game : { ...game, has_denuvo: flag };
          })
        );
      })
      .catch((error) => console.warn('Denuvo enrichment failed:', error));
    return () => {
      cancelled = true;
    };
    // pageKey is the complete enrichment request identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, pageKey]);

  const runSearch = async (query: string) => {
    setPage(1);
    if (!query.trim()) {
      clear();
      trending.reset();
      return;
    }
    await search(query);
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
    searchError: error,
    runSearch,
    clearSearch,
  };
}
