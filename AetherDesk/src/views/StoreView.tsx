import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { SpecificVersionModal, LuaManifestRow } from '../modals/SpecificVersionModal';
import { GameInfoModal } from '../modals/GameInfoModal';
import { preloadGameCovers } from '../ui/GameCover';
import { GameCard } from '../ui/GameCard';
import { StatusAlert } from '../ui/StatusAlert';
import { useStoreSearch, StoreGameResult as StoreGame } from '../hooks/useStoreSearch';
import { enrichDenuvoFlags } from '../hooks/useDenuvoEnrichment';
import { emptyStatus, StatusMessage } from '../types/ui';
import { getSettings } from '../hooks/useSettings';


interface StoreViewProps {
  onRefreshUsage?: (forcedKey?: string) => Promise<void>;
  isActive: boolean;
  settingsRevision: number;
}

export const StoreView = ({ onRefreshUsage, isActive, settingsRevision }: StoreViewProps) => {
  const [searchQuery, setSearchQuery] = useState('');
  const [page, setPage] = useState(1);
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

  // Selected key/manifest source state ('hubcap', 'ryuu', 'oureveryday', 'local')
  const [selectedSource, setSelectedSource] = useState<'hubcap' | 'ryuu' | 'oureveryday' | 'local'>('oureveryday');

  // Status message for download operations inside the modal
  const [downloadStatus, setDownloadStatus] = useState<StatusMessage>(emptyStatus());
  const [isDownloading, setIsDownloading] = useState(false);

  // Specific-version editor state. The normal download modal closes before this modal opens.
  const [versionGame, setVersionGame] = useState<StoreGame | null>(null);
  const [manifestRows, setManifestRows] = useState<LuaManifestRow[]>([]);

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

  const handleSearch = async (e: React.FormEvent) => {
    e.preventDefault();
    setPage(1);
    try {
      if (!searchQuery.trim()) {
        clear();
        trendingRequests.current.clear();
        nextTrendingStart.current = 0;
        hasActivatedStoreFront.current = false;
        await loadTrendingGames(0, itemsPerPage);
        return;
      }
      await search(searchQuery);
    } catch (err: any) {
      alert(`Search error: ${err}`);
    }
  };

  const handleDownloadSteam = async () => {
    if (!selectedGame) return;

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
      const command = selectedSource === 'ryuu' ? 'trigger_ryuu_download' : 'trigger_hubcap_download';
      const result: string = await invoke(command, {
        appId: Number(selectedGame.appId),
        apiKey: apiKeyToUse,
        steamPath: steamPathToUse
      });

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

      const command = selectedSource === 'ryuu' ? 'prepare_ryuu_specific_version_download' : 'prepare_specific_version_download';
      const rows: LuaManifestRow[] = await invoke(command, {
        appId: Number(selectedGame.appId),
        apiKey: apiKeyToUse,
        steamPath: steamPathToUse
      });

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
        <input
          type="text"
          placeholder="Search games by name or App ID on Steam..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          className="store-search-input"
        />
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

      {/* 10 rows x 2 columns Grid */}
      <div className="store-grid">
        {isLoading || (isTrendingLoading && pageGames.length === 0) ? (
          <div className="store-no-results">
            {isLoading ? 'Loading results from Steam & Hubcap...' : 'Loading trending Steam games...'}
          </div>
        ) : pageGames.length > 0 ? (
          pageGames.map((game) => (
            <GameCard
              key={game.id}
              game={game}
              actions={[
                {
                  label: 'Download',
                  variant: 'primary',
                  onClick: async (selected) => {
                    setSelectedGame(selected);
                    setDownloadStatus({ text: '', type: 'info' });
                    setIsDownloading(false);

                    // Carica impostazioni per verificare se c'è una chiave Hubcap valida
                    try {
                      const settings = await getSettings();
                      const hasValidHubcapKey = settings.hubcap_api_key?.trim() !== '';

                      if (hasValidHubcapKey && selected.has_manifest) {
                        setSelectedSource('hubcap');
                      } else {
                        setSelectedSource('oureveryday');
                      }
                    } catch (err) {
                      // Fallback a oureveryday se c'è un errore
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
        <div className="modal-overlay">
          <div className="modal-container">
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
              <StatusAlert status={downloadStatus} style={{ padding: '10px 15px', fontSize: '12px' }} />

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
                    onClick={() => setSelectedSource('ryuu')}
                    className={`source-btn ${selectedSource === 'ryuu' ? 'active' : ''}`}
                  >
                    Ryuu
                  </button>
                  <button
                    disabled={isDownloading}
                    onClick={() => setSelectedSource('oureveryday')}
                    className={`source-btn ${selectedSource === 'oureveryday' ? 'active' : ''}`}
                  >
                    OurEveryday
                  </button>
                  <button
                    disabled={true}
                    title="Local download is not available yet"
                    className="source-btn"
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

      {/* Reusable specific-version modal. The same component can be opened later from Library/Installed games. */}
      {versionGame && (
        <SpecificVersionModal
          game={versionGame}
          initialRows={manifestRows}
          onClose={() => {
            setVersionGame(null);
            setManifestRows([]);
          }}
        />
      )}
    </div>
  );
};
