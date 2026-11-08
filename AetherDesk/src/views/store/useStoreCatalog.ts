import { useEffect, useRef, useState } from 'react';
import { enrichDenuvoFlags } from '../../hooks/useDenuvoEnrichment';
import { enrichUco2Flags } from '../../hooks/useUco2Enrichment';
import { enrichCrackStatus } from '../../hooks/useCrackStatusEnrichment';
import { enrichPcDrm } from '../../hooks/usePcDrmEnrichment';
import { enrichGenres } from '../../hooks/useGenreEnrichment';
import { useStoreSearch } from '../../hooks/useStoreSearch';
import { preloadGameCovers } from '../../ui/GameCover';
import { useTrendingStore } from './useTrendingStore';
import { useStoreBadgeFilter, matchesStoreBadgeFilter } from './useStoreBadgeFilter';
import { STORE_ITEMS_PER_PAGE, compareStoreGames, type StoreSortMode } from '../../constants/store';

interface Params {
  /** Hidden views stay mounted; only the visible tab may start network work. */
  active: boolean;
  settingsRevision: number;
  settingsReady: boolean;
}

export function useStoreCatalog({ active, settingsRevision, settingsReady }: Params) {
  const [page, setPage] = useState(1);
  const [sortMode, setSortMode] = useState<StoreSortMode>('newest');
  const [genreFilter, setGenreFilter] = useState<string>('');
  const [storeFrontFilter, setStoreFrontFilter] = useState<string>('latest');
  const { badgeFilter, setBadgeFilter, STORE_BADGE_FILTERS } = useStoreBadgeFilter();
  const observedSettingsRevision = useRef<number | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const storeSearch = useStoreSearch();
  const trending = useTrendingStore(storeSearch.setResults, storeFrontFilter, sortMode);
  const { results, setResults, isLoading, hasSearched, activeQuery, error, search, clear } = storeSearch;

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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, settingsRevision, settingsReady]);

  // Enrich genres for any result that doesn't have them yet.
  useEffect(() => {
    const missing = results.filter((g) => !g.genres);
    if (!active || missing.length === 0) return;
    let cancelled = false;
    enrichGenres(missing)
      .then((enriched) => {
        if (cancelled) return;
        const map = new Map(enriched.map((g) => [String(g.appId), g.genres]));
        setResults((current) =>
          current.map((g) => {
            const genres = map.get(String(g.appId));
            return genres === undefined ? g : { ...g, genres };
          })
        );
      })
      .catch((error) => console.warn('Genre enrichment failed:', error));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, results.length]);

  const prevStoreFrontFilter = useRef(storeFrontFilter);
  // Reset trending session when the storefront filter changes.
  useEffect(() => {
    if (prevStoreFrontFilter.current === storeFrontFilter) return;
    prevStoreFrontFilter.current = storeFrontFilter;
    setPage(1);
    clear();
    trending.reset();
    void trending.loadNext(STORE_ITEMS_PER_PAGE * 2);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [storeFrontFilter]);

  const availableGenres = Array.from(new Set(results.flatMap((g) => g.genres || []))).sort();

  const genreFiltered = results.filter((g) => (genreFilter ? (g.genres || []).includes(genreFilter) : true));
  const badgeFiltered = genreFiltered.filter((g) => matchesStoreBadgeFilter(g, badgeFilter));
  // Browse mode keeps the fetch order: each incoming batch is ordered on
  // arrival, so sorting the whole accumulated catalog here would move games
  // between pages every time a later offset loads. Search replaces the list
  // instead of appending, so a full sort is safe there.
  const filteredSortedResults = activeQuery.trim()
    ? [...badgeFiltered].sort((a, b) => compareStoreGames(a, b, sortMode))
    : badgeFiltered;

  const totalPages = Math.ceil(filteredSortedResults.length / STORE_ITEMS_PER_PAGE) || 1;
  const startIndex = (page - 1) * STORE_ITEMS_PER_PAGE;
  const pageGames = filteredSortedResults.slice(startIndex, startIndex + STORE_ITEMS_PER_PAGE);
  const pageKey = pageGames.map((game) => game.appId).join(',');

  useEffect(() => {
    if (!active || pageGames.length === 0 || document.visibilityState !== 'visible') return;
    preloadGameCovers(
      pageGames.map((game) => ({ appId: game.appId, imageUrl: game.imageUrl })),
      pageGames.length
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, pageKey]);

  useEffect(() => {
    if (!active || !settingsReady || activeQuery.trim()) return;
    if (results.length < (page + 1) * STORE_ITEMS_PER_PAGE) {
      void trending.loadNext(STORE_ITEMS_PER_PAGE * 2);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, page, results.length, activeQuery, settingsReady]);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
  }, [page, activeQuery, sortMode, genreFilter]);

  const prevSortMode = useRef(sortMode);
  // An explicit sort change reorders the loaded catalog once. Batches loaded
  // afterwards still append, so pages stay put after this reflow.
  useEffect(() => {
    if (prevSortMode.current === sortMode) return;
    prevSortMode.current = sortMode;
    if (activeQuery.trim()) return;
    setResults((current) => [...current].sort((a, b) => compareStoreGames(a, b, sortMode)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sortMode]);

  // A genre filter or a reset can shrink the catalog under the current page.
  useEffect(() => {
    if (page > totalPages) setPage(totalPages);
  }, [page, totalPages]);

  useEffect(() => {
    if (!active || pageGames.length === 0) return;

    let cancelled = false;
    enrichDenuvoFlags(pageGames)
      .then((enriched) => {
        if (cancelled) return;
        const enrichedMap = new Map(enriched.map((game) => [String(game.appId), game]));
        setResults((current) =>
          current.map((game) => {
            const updated = enrichedMap.get(String(game.appId));
            return updated === undefined
              ? game
              : {
                  ...game,
                  has_denuvo: updated.has_denuvo,
                  drm_protection: updated.drm_protection ?? game.drm_protection,
                };
          })
        );
      })
      .catch((error) => console.warn('Denuvo enrichment failed:', error));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, pageKey]);

  useEffect(() => {
    if (!active || pageGames.length === 0) return;
    let cancelled = false;
    enrichUco2Flags(pageGames)
      .then((enriched) => {
        if (cancelled) return;
        const flags = new Map(enriched.map((game) => [String(game.appId), game.uco2_supported]));
        setResults((current) =>
          current.map((game) => {
            const flag = flags.get(String(game.appId));
            return flag === undefined ? game : { ...game, uco2_supported: flag };
          })
        );
      })
      .catch((error) => console.warn('UCO2 enrichment failed:', error));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pageKey]);

  useEffect(() => {
    if (!active || pageGames.length === 0) return;
    let cancelled = false;
    enrichCrackStatus(pageGames)
      .then((enriched) => {
        if (cancelled) return;
        const statusMap = new Map(enriched.map((game) => [String(game.appId), game.crack_status]));
        const drmMap = new Map(enriched.map((game) => [String(game.appId), game.drm_protection]));
        setResults((current) =>
          current.map((game) => {
            const status = statusMap.get(String(game.appId));
            const drm = drmMap.get(String(game.appId));
            return {
              ...game,
              ...(status === undefined ? {} : { crack_status: status }),
              ...(drm === undefined ? {} : { drm_protection: drm }),
            };
          })
        );
      })
      .catch((error) => console.warn('Crack status enrichment failed:', error));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pageKey]);

  useEffect(() => {
    if (!active || pageGames.length === 0) return;
    let cancelled = false;
    enrichPcDrm(pageGames)
      .then((enriched) => {
        if (cancelled) return;
        const map = new Map(enriched.map((game) => [String(game.appId), game.drm_protection]));
        setResults((current) =>
          current.map((game) => {
            const drm = map.get(String(game.appId));
            return drm === undefined ? game : { ...game, drm_protection: drm };
          })
        );
      })
      .catch((error) => console.warn('PC DRM enrichment failed:', error));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pageKey]);

  // Background refresh must not yank the user back to page 1; the explicit
  // Refresh button resets, the interval only tops up content.
  const refreshCatalog = (reset = true) => {
    if (reset) {
      setPage(1);
      clear();
      trending.reset();
    }
    void trending.loadNext(STORE_ITEMS_PER_PAGE * 2);
  };

  // Live update: periodically refresh the trending catalog so new releases
  // appear without a manual reload. Only while the store is in browse mode
  // (no active search) and the tab is visible.
  useEffect(() => {
    if (!active || activeQuery.trim()) return;
    const id = window.setInterval(
      () => {
        if (document.visibilityState !== 'visible') return;
        refreshCatalog(false);
      },
      3 * 60 * 1000
    );
    return () => window.clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, activeQuery, settingsReady]);

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
      console.warn('Store search failed:', error);
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
    resultsLength: results.length,
    isLoading,
    isTrendingLoading: trending.isLoading,
    hasSearched,
    activeQuery,
    searchError: error,
    runSearch,
    clearSearch,
    refreshCatalog,
    sortMode,
    setSortMode,
    genreFilter,
    setGenreFilter,
    availableGenres,
    storeFrontFilter,
    setStoreFrontFilter,
    badgeFilter,
    setBadgeFilter,
    STORE_BADGE_FILTERS,
  };
}
