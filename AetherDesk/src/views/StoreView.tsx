import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { SearchSuggest, moveSuggestIndex } from '../ui/SearchSuggest';
import { useSteamSuggest } from '../hooks/useSteamSuggest';
import { LuaManifestRow } from '../modals/SpecificVersionModal';
import ChangeVersionModal from '../modals/ChangeVersionModal';
import { GameInfoModal } from '../modals/GameInfoModal';
import { LocalDownloadModal } from '../modals/LocalDownloadModal';
import { preloadGameCovers } from '../ui/GameCover';
import { GameCard } from '../ui/GameCard';
import { StatusAlert } from '../ui/StatusAlert';
import { useStoreSearch, StoreGameResult as StoreGame } from '../hooks/useStoreSearch';
import { enrichDenuvoFlags } from '../hooks/useDenuvoEnrichment';
import { useModalDismiss } from '../hooks/useModalDismiss';
import { emptyStatus, StatusMessage } from '../types/ui';
import { getSettings } from '../hooks/useSettings';


interface StoreViewProps {
  onRefreshUsage?: (forcedKey?: string) => Promise<void>;
  isActive: boolean;
  settingsRevision: number;
  useAlternativeGameCards: boolean;
  alternativeCardsOpacity: number;
  alternativeCardsFade: number;
}

export const StoreView = ({ onRefreshUsage, isActive, settingsRevision, useAlternativeGameCards, alternativeCardsOpacity, alternativeCardsFade }: StoreViewProps) => {
  const [searchQuery, setSearchQuery] = useState('');
  const [isSuggestOpen, setIsSuggestOpen] = useState(false);
  const [activeSuggestIndex, setActiveSuggestIndex] = useState<number | null>(null);
  const searchPanelRef = useRef<HTMLDivElement | null>(null);
  const [page, setPage] = useState(1);
  const { items: suggestItems, isLoading: isSuggestLoading } = useSteamSuggest(searchQuery, isSuggestOpen);
  const itemsPerPage = 20; // 10 rows * 2 columns = 20 items per page
  const {
    results: storeGames,
    setResults,
    isLoading,
    hasSearched,
    activeQuery,
    search,
    clear,
  } = useStoreSearch();
  const [isTrendingLoading, setIsTrendingLoading] = useState(false);
  const trendingRequests = useRef<Set<number>>(new Set());
  const nextTrendingStart = useRef(0);
  const hasActivatedStoreFront = useRef(false);
  const observedSettingsRevision = useRef(settingsRevision);

  // Active game selected for download modal, null means modal is closed
  const [selectedGame, setSelectedGame] = useState<StoreGame | null>(null);
  const [infoGame, setInfoGame] = useState<StoreGame | null>(null);

  // Selected manifest source. LuaTools uses its own authenticated session;
  // Hubcap/Ryuu use the API keys configured in Settings.
  const [selectedSource, setSelectedSource] = useState<'hubcap' | 'luatools' | 'ryuu' | 'oureveryday' | 'local'>('oureveryday');

  // Status message for download operations inside the modal
  const [downloadStatus, setDownloadStatus] = useState<StatusMessage>(emptyStatus());
  const [isDownloading, setIsDownloading] = useState(false);

  // ESC chiude il modal di download (uniforme con gli altri popup); il click
  // fuori è gestito dall'overlay. Entrambi bloccati durante un download.
  useModalDismiss(() => setSelectedGame(null), isDownloading);

  // Specific-version editor state. The normal download modal closes before this modal opens.
  const [versionGame, setVersionGame] = useState<StoreGame | null>(null);
  const [manifestRows, setManifestRows] = useState<LuaManifestRow[]>([]);

  // Local-install modal state. Opens on top of the download modal when the
  // "Local" source is picked; the download modal stays open underneath.
  const [localGame, setLocalGame] = useState<StoreGame | null>(null);

  const mergeTrendingGames = (incoming: StoreGame[]) => {
    setResults((prev) => {
      const seen = new Set(prev.map((game) => Number(game.id)));
      const merged = [...prev];
      for (const game of incoming) {
        if (!seen.has(Number(game.id))) {
          seen.add(Number(game.id));
          merged.push(game);
        }
      }
      return merged;
    });
  };

  const loadTrendingGames = async (start: number, count: number) => {
    if (trendingRequests.current.has(start)) return;
    trendingRequests.current.add(start);
    nextTrendingStart.current = Math.max(nextTrendingStart.current, start + count);
    setIsTrendingLoading(true);
    try {
      const games: StoreGame[] = await invoke('get_trending_store_games', { start, count });
      mergeTrendingGames(games || []);
    } catch (err) {
      console.warn('Trending store preload failed:', err);
    } finally {
      setIsTrendingLoading(false);
    }
  };

  useEffect(() => {
    loadTrendingGames(0, itemsPerPage);
  }, []);

  useEffect(() => {
    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (target && !searchPanelRef.current?.contains(target)) {
        setIsSuggestOpen(false);
      }
    };
    document.addEventListener('mousedown', handlePointerDown);
    return () => document.removeEventListener('mousedown', handlePointerDown);
  }, []);

  useEffect(() => {
    setActiveSuggestIndex(null);
  }, [searchQuery]);

  useEffect(() => {
    if (!isActive || hasActivatedStoreFront.current || activeQuery.trim()) return;
    hasActivatedStoreFront.current = true;
    loadTrendingGames(itemsPerPage, itemsPerPage);
  }, [isActive, activeQuery]);

  useEffect(() => {
    if (observedSettingsRevision.current === settingsRevision) return;
    observedSettingsRevision.current = settingsRevision;
    setPage(1);

    if (activeQuery.trim()) {
      search(activeQuery).catch((err) => console.warn('Store search refresh after settings save failed:', err));
      return;
    }

    clear();
    trendingRequests.current.clear();
    nextTrendingStart.current = 0;
    hasActivatedStoreFront.current = false;
    loadTrendingGames(0, itemsPerPage);
    if (isActive) {
      hasActivatedStoreFront.current = true;
      loadTrendingGames(itemsPerPage, itemsPerPage);
    }
  }, [settingsRevision]);

  // Pagination calculation
  const totalPages = Math.ceil(storeGames.length / itemsPerPage) || 1;
  const startIndex = (page - 1) * itemsPerPage;
  const pageGames = storeGames.slice(startIndex, startIndex + itemsPerPage);

  // Per-page Denuvo enrichment: only the ~20 currently visible games are
  // checked, never the whole result set. The backend layers a 30-day disk
  // cache + 429 circuit breaker on top, so Steam's rate limit cannot be hit
  // through normal browsing (used to be one appdetails call per result).
  const pageKey = pageGames.map((game) => game.appId).join(',');
  useEffect(() => {
    if (pageGames.length > 0) {
      preloadGameCovers(pageGames.map((game) => ({ appId: game.appId, imageUrl: game.imageUrl })), pageGames.length);
    }
  }, [pageKey]);

  useEffect(() => {
    if (!isActive || activeQuery.trim() || storeGames.length === 0) return;
    const loadedPages = Math.ceil(storeGames.length / itemsPerPage);
    if (page >= loadedPages) {
      loadTrendingGames(nextTrendingStart.current, itemsPerPage);
    }
  }, [page, storeGames.length, activeQuery]);

  useEffect(() => {
    if (pageGames.length === 0 || !activeQuery.trim()) return;
    let cancelled = false;
    enrichDenuvoFlags(pageGames)
      .then((enriched) => {
        if (cancelled) return;
        const flags = new Map(enriched.map((g) => [String(g.appId), g.has_denuvo]));
        setResults((prev) =>
          prev.map((game) => {
            const flag = flags.get(String(game.appId));
            return flag === undefined ? game : { ...game, has_denuvo: flag };
          })
        );
      })
      .catch((err) => console.warn('Denuvo enrichment failed:', err));
    return () => {
      cancelled = true;
    };
  }, [pageKey]);

  const runSearch = async (query: string) => {
    setSearchQuery(query);
    setPage(1);
    setIsSuggestOpen(false);
    setActiveSuggestIndex(null);
    try {
      if (!query.trim()) {
        clear();
        trendingRequests.current.clear();
        nextTrendingStart.current = 0;
        hasActivatedStoreFront.current = false;
        await loadTrendingGames(0, itemsPerPage);
        return;
      }
      await search(query);
    } catch (err: any) {
      alert(`Search error: ${err}`);
    }
  };

  const handleSearch = async (e: React.FormEvent) => {
    e.preventDefault();
    if (isSuggestOpen && activeSuggestIndex !== null && suggestItems[activeSuggestIndex]) {
      await runSearch(suggestItems[activeSuggestIndex].name);
      return;
    }
    await runSearch(searchQuery);
  };

  const handleDownloadSteam = async () => {
    if (!selectedGame) return;

    if (selectedSource === 'local') {
      setDownloadStatus({ text: 'The Local source installs from your own files: click the Local button above to open its popup.', type: 'error' });
      return;
    }

    setIsDownloading(true);
    setDownloadStatus({ text: 'Initializing pipeline...', type: 'info' });

    try {
      // 1. Load active settings from Rust (to get current API key and Steam Path)
      setDownloadStatus({ text: 'Loading local configurations...', type: 'info' });
      const settings = await getSettings();

      const apiKeyToUse = selectedSource === 'hubcap' ? settings.hubcap_api_key : selectedSource === 'ryuu' ? settings.ryuu_api_key : 'oureveryday_public';
      const steamPathToUse = settings.steam_path;

      if (selectedSource === 'hubcap' && (!apiKeyToUse || apiKeyToUse.trim() === '')) {
        setDownloadStatus({ text: 'Error: Please enter your Hubcap API Key in Settings first!', type: 'error' });
        setIsDownloading(false);
        return;
      }
      if (selectedSource === 'ryuu' && (!apiKeyToUse || apiKeyToUse.trim() === '')) {
        setDownloadStatus({ text: 'Error: Please enter your Ryuu API Key in Settings first!', type: 'error' });
        setIsDownloading(false);
        return;
      }
      if (!steamPathToUse || steamPathToUse.trim() === '') {
        setDownloadStatus({ text: 'Error: Please specify the Steam path in Settings first!', type: 'error' });
        setIsDownloading(false);
        return;
      }

      // 2. Invoke the decoupled, professional Rust download orchestrator!
      setDownloadStatus({ text: `Connecting to source ${selectedSource.toUpperCase()}...`, type: 'info' });
      const command = selectedSource === 'luatools'
        ? 'trigger_luatools_download'
        : selectedSource === 'ryuu'
          ? 'trigger_ryuu_download'
          : 'trigger_hubcap_download';
      const args = selectedSource === 'luatools'
        ? { appId: Number(selectedGame.appId), steamPath: steamPathToUse }
        : { appId: Number(selectedGame.appId), apiKey: apiKeyToUse, steamPath: steamPathToUse };
      const result: string = await invoke(command, args);

      setDownloadStatus({ text: result, type: 'success' });
      setIsDownloading(false);
      onRefreshUsage?.();

      // Auto close modal after a short delay on success
      setTimeout(() => {
        setSelectedGame(null);
        setDownloadStatus({ text: '', type: 'info' });
      }, 3000);

    } catch (err: any) {
      setDownloadStatus({ text: `Download failed: ${err}`, type: 'error' });
      setIsDownloading(false);
    }
  };

  const handleDownloadOlder = async () => {
    if (!selectedGame) return;

    if (selectedSource === 'local') {
      setDownloadStatus({ text: 'The Local source installs from your own files: click the Local button above to open its popup.', type: 'error' });
      return;
    }

    setIsDownloading(true);
    setDownloadStatus({ text: 'Downloading Lua and preparing version table...', type: 'info' });

    try {
      const settings = await getSettings();
      const apiKeyToUse = selectedSource === 'hubcap' ? settings.hubcap_api_key : selectedSource === 'ryuu' ? settings.ryuu_api_key : 'oureveryday_public';
      const steamPathToUse = settings.steam_path;

      if (selectedSource === 'hubcap' && (!apiKeyToUse || apiKeyToUse.trim() === '')) {
        setDownloadStatus({ text: 'Error: Please enter your Hubcap API Key in Settings first!', type: 'error' });
        setIsDownloading(false);
        return;
      }
      if (selectedSource === 'ryuu' && (!apiKeyToUse || apiKeyToUse.trim() === '')) {
        setDownloadStatus({ text: 'Error: Please enter your Ryuu API Key in Settings first!', type: 'error' });
        setIsDownloading(false);
        return;
      }
      if (!steamPathToUse || steamPathToUse.trim() === '') {
        setDownloadStatus({ text: 'Error: Please specify the Steam path in Settings first!', type: 'error' });
        setIsDownloading(false);
        return;
      }

      const command = selectedSource === 'luatools'
        ? 'prepare_luatools_specific_version_download'
        : selectedSource === 'ryuu'
          ? 'prepare_ryuu_specific_version_download'
          : 'prepare_specific_version_download';
      const args = selectedSource === 'luatools'
        ? { appId: Number(selectedGame.appId), steamPath: steamPathToUse }
        : { appId: Number(selectedGame.appId), apiKey: apiKeyToUse, steamPath: steamPathToUse };
      const rows: LuaManifestRow[] = await invoke(command, args);

      setManifestRows((rows || []).map(row => ({ ...row, manifestInput: '' })));
      setVersionGame(selectedGame);

      // Close the download modal and open the reusable version picker modal.
      setSelectedGame(null);
      setDownloadStatus({ text: '', type: 'info' });
      setIsDownloading(false);
      onRefreshUsage?.();
    } catch (err: any) {
      setDownloadStatus({ text: `Specific version setup failed: ${err}`, type: 'error' });
      setIsDownloading(false);
    }
  };


  return (
    <div className="store-view">
      {/* Upper header section */}
      <div className="store-header">
        <h1 className="store-title">Store</h1>
        <p className="store-subtitle">Browse, search and unlock game manifests using AetherDesk's built-in database.</p>
      </div>

      {/* Separator line */}
      <div className="store-separator"></div>

      {/* Search Input Area */}
      <form onSubmit={handleSearch} className="store-search-form">
        <div className="home-search-wrapper store-suggest-wrap" ref={searchPanelRef}>
          <input
            type="text"
            placeholder="Search games by name or App ID on Steam..."
            value={searchQuery}
            onChange={(e) => {
              setSearchQuery(e.target.value);
              setIsSuggestOpen(true);
              setActiveSuggestIndex(null);
            }}
            onFocus={() => setIsSuggestOpen(true)}
            onKeyDown={(event) => {
              if (event.key === 'ArrowDown') {
                event.preventDefault();
                setIsSuggestOpen(true);
                setActiveSuggestIndex((prev) => moveSuggestIndex(prev, suggestItems.length, 1));
              } else if (event.key === 'ArrowUp') {
                event.preventDefault();
                setIsSuggestOpen(true);
                setActiveSuggestIndex((prev) => moveSuggestIndex(prev, suggestItems.length, -1));
              } else if (event.key === 'Escape') {
                setIsSuggestOpen(false);
              }
            }}
            className="store-search-input"
          />
          {searchQuery && (
            <button
              type="button"
              className="home-search-clear"
              aria-label="Clear search"
              onClick={() => {
                setSearchQuery('');
                setIsSuggestOpen(false);
                setActiveSuggestIndex(null);
                clear();
                trendingRequests.current.clear();
                nextTrendingStart.current = 0;
                hasActivatedStoreFront.current = false;
                loadTrendingGames(0, itemsPerPage);
              }}
            >
              &times;
            </button>
          )}
          <SearchSuggest
            open={isSuggestOpen && searchQuery.trim().length >= 2}
            items={suggestItems}
            emptyText="No Steam suggestions."
            statusText={isSuggestLoading ? 'Searching Steam…' : undefined}
            activeIndex={activeSuggestIndex}
            onHoverIndex={setActiveSuggestIndex}
            onSelect={(item) => { void runSearch(item.name); }}
          />
        </div>
        <button type="submit" className="store-search-btn" disabled={isLoading} title="Search Catalog">
          {isLoading ? (
            <span style={{ fontSize: '12px' }}>...</span>
          ) : (
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" style={{ display: 'block' }}>
              <circle cx="11" cy="11" r="8"></circle>
              <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
            </svg>
          )}
        </button>
      </form>

      {/* Separator line */}
      <div className="store-separator"></div>

      {/* 10 rows x 2 columns Grid. The alternative card layout reads the
          opacity/fade values from CSS variables set here (live preview). */}
      <div
        className={useAlternativeGameCards ? 'store-grid alt-card-grid' : 'store-grid'}
        style={useAlternativeGameCards ? {
          '--alt-card-opacity': Math.max(0, Math.min(100, alternativeCardsOpacity)),
          '--alt-card-fade': Math.max(0, Math.min(100, alternativeCardsFade)),
        } as React.CSSProperties : undefined}
      >
        {isLoading || (isTrendingLoading && pageGames.length === 0) ? (
          <div className="store-no-results">
            {isLoading ? 'Loading results from Steam & Hubcap...' : 'Loading trending Steam games...'}
          </div>
        ) : pageGames.length > 0 ? (
          pageGames.map((game) => (
            <GameCard
              key={game.id}
              game={game}
              cardVariant={useAlternativeGameCards ? 'backdrop' : 'classic'}
              actions={[
                {
                  label: 'Download',
                  variant: 'primary',
                  onClick: async (selected) => {
                    setSelectedGame(selected);
                    setDownloadStatus({ text: '', type: 'info' });
                    setIsDownloading(false);

                    // Prefer Hubcap when a key is configured and a manifest exists.
                    try {
                      const settings = await getSettings();
                      const hasValidHubcapKey = settings.hubcap_api_key?.trim() !== '';

                      if (hasValidHubcapKey && selected.has_manifest) {
                        setSelectedSource('hubcap');
                      } else {
                        setSelectedSource('oureveryday');
                      }
                    } catch (err) {
                      // Fall back to MOED if settings cannot be read.
                      setSelectedSource('oureveryday');
                    }
                  },
                },
                {
                  label: 'Info',
                  variant: 'secondary',
                  onClick: setInfoGame,
                },
              ]}
            />
          ))
        ) : (
          <div className="store-no-results">
            {hasSearched ? `No games found for "${activeQuery}"` : 'Enter a query above to search the Steam catalog.'}
          </div>
        )}
      </div>

      {/* Pagination controls below the grid */}
      {!isLoading && totalPages > 1 && (
        <div className="store-pagination">
          <button
            disabled={page === 1}
            onClick={() => setPage(prev => Math.max(prev - 1, 1))}
            className="pagination-btn"
          >
            &larr; Prev
          </button>
          <span className="pagination-info">
            Page {page} of {totalPages}
          </span>
          <button
            disabled={page === totalPages}
            onClick={() => setPage(prev => Math.min(prev + 1, totalPages))}
            className="pagination-btn"
          >
            Next &rarr;
          </button>
        </div>
      )}

      {infoGame && (
        <GameInfoModal
          appId={Number(infoGame.appId)}
          fallbackName={infoGame.name}
          fallbackImageUrl={infoGame.imageUrl}
          onClose={() => setInfoGame(null)}
        />
      )}

      {/* DYNAMIC DOWNLOAD MODAL / POPUP */}
      {selectedGame && (
        <div className="modal-overlay" onClick={isDownloading ? undefined : () => setSelectedGame(null)}>
          <div className="modal-container" onClick={(e) => e.stopPropagation()}>
            {/* Header: Title + Close Button */}
            <div className="modal-header">
              <span className="modal-title">
                Download: <strong style={{ color: '#ffffff' }}>{selectedGame.name}</strong> ({selectedGame.appId})
              </span>
              <button
                onClick={() => {
                  if (!isDownloading) {
                    setSelectedGame(null);
                  }
                }}
                className="modal-close-btn"
                disabled={isDownloading}
                style={{ opacity: isDownloading ? 0.3 : 1 }}
              >
                &times;
              </button>
            </div>

            {/* Separator line */}
            <div className="modal-separator"></div>

            {/* Modal Body Content */}
            <div className="modal-body">
              {/* Operation Feedback inside the Modal */}
              <StatusAlert status={downloadStatus} className="settings-alert--compact" />

              {/* Dark Source Box Panel */}
              <div className="source-box">
                <span className="source-label">Source:</span>
                <div className="source-buttons-row">
                  <button
                    disabled={isDownloading}
                    onClick={() => setSelectedSource('hubcap')}
                    className={`source-btn ${selectedSource === 'hubcap' ? 'active' : ''}`}
                  >
                    Hubcap
                  </button>
                  <button
                    disabled={isDownloading}
                    onClick={() => setSelectedSource('luatools')}
                    className={`source-btn ${selectedSource === 'luatools' ? 'active' : ''}`}
                    title="Uses your signed-in lua.tools account and automatically selects an available source"
                  >
                    LuaTools
                  </button>
                  <button
                    disabled={isDownloading}
                    onClick={() => setSelectedSource('ryuu')}
                    className={`source-btn ${selectedSource === 'ryuu' ? 'active' : ''}`}
                  >
                    Ryuu
                  </button>
                  <button
                    disabled={isDownloading}
                    onClick={() => setSelectedSource('oureveryday')}
                    className={`source-btn ${selectedSource === 'oureveryday' ? 'active' : ''}`}
                    title="MOED manifest source"
                  >
                    MOED
                  </button>
                  <button
                    disabled={isDownloading}
                    onClick={() => {
                      setSelectedSource('local');
                      // Open the Local install popup (same layout as Apply
                      // Crack, without the option checkboxes).
                      setLocalGame(selectedGame);
                    }}
                    title="Install game files from a local archive or loose files"
                    className={`source-btn ${selectedSource === 'local' ? 'active' : ''}`}
                  >
                    Local
                  </button>
                </div>
              </div>

              {/* Action 1: Download Latest Version Button */}
              <button
                onClick={handleDownloadSteam}
                className="big-action-btn"
                disabled={isDownloading}
                style={{ opacity: isDownloading ? 0.5 : 1 }}
              >
                <div className="action-icon">⚡</div>
                <div className="action-info">
                  <span className="action-title">Download Latest Version</span>
                  <span className="action-desc">
                    Downloads the most recent manifest files and decryption keys directly into Steam. This allows Steam to download and install the latest official release.
                  </span>
                </div>
              </button>

              {/* Action 2: Download Specific Version Button */}
              <button
                onClick={handleDownloadOlder}
                className="big-action-btn"
                disabled={isDownloading}
                style={{ opacity: isDownloading ? 0.5 : 1 }}
              >
                <div className="action-icon">📦</div>
                <div className="action-info">
                  <span className="action-title">Download Specific Version</span>
                  <span className="action-desc">
                    Allows you to pick and download a specific historical version or downgrade release of the game by pinning custom Steam manifest IDs.
                  </span>
                </div>
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Change Version modal (Manual / Auto / Builds). Also reachable from Library/Installed games. */}
      {versionGame && (
        <ChangeVersionModal
          game={versionGame}
          initialRows={manifestRows}
          onClose={() => {
            setVersionGame(null);
            setManifestRows([]);
          }}
        />
      )}

      {/* Local install modal: opened from the "Local" source button inside the
          download modal. Rendered last so it stacks on top of it. */}
      {localGame && (
        <LocalDownloadModal
          game={{ name: localGame.name, appId: localGame.appId }}
          onClose={() => setLocalGame(null)}
          onInstalled={() => {
            setLocalGame(null);
            setSelectedGame(null);
            setDownloadStatus({ text: '', type: 'info' });
            onRefreshUsage?.();
          }}
        />
      )}
    </div>
  );
};
