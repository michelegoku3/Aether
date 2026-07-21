import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface StoreGame {
  id: number;
  name: string;
  appId: string;
  has_manifest: boolean;
}

export const StoreView = () => {
  const [searchQuery, setSearchQuery] = useState('');
  const [activeQuery, setActiveQuery] = useState('');
  const [page, setPage] = useState(1);
  const itemsPerPage = 20; // 10 rows * 2 columns = 20 items per page

  // Dynamic store games list loaded from the Rust backend
  const [storeGames, setStoreGames] = useState<StoreGame[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [hasSearched, setHasSearched] = useState(false);

  // Active game selected for download modal, null means modal is closed
  const [selectedGame, setSelectedGame] = useState<StoreGame | null>(null);
  
  // Selected key/manifest source state ('hubcap', 'oureveryday', 'local')
  const [selectedSource, setSelectedSource] = useState<'hubcap' | 'oureveryday' | 'local'>('oureveryday');

  // Status message for download operations inside the modal
  const [downloadStatus, setDownloadStatus] = useState({ text: '', type: 'info' });
  const [isDownloading, setIsDownloading] = useState(false);

  // Pagination calculation
  const totalPages = Math.ceil(storeGames.length / itemsPerPage) || 1;
  const startIndex = (page - 1) * itemsPerPage;
  const pageGames = storeGames.slice(startIndex, startIndex + itemsPerPage);

  const handleSearch = async (e: React.FormEvent) => {
    e.preventDefault();
    
    // If query is empty, clear results and restore default state
    if (!searchQuery.trim()) {
      setStoreGames([]);
      setHasSearched(false);
      return;
    }

    setIsLoading(true);
    setHasSearched(true);
    setPage(1);

    try {
      // Invoke the high-performance unified parallel search in Rust!
      const results: any = await invoke('search_store', { query: searchQuery });
      setStoreGames(results || []);
      setActiveQuery(searchQuery);
      setIsLoading(false);
    } catch (err: any) {
      alert(`Search error: ${err}`);
      setIsLoading(false);
    }
  };

  const handleDownloadSteam = async () => {
    if (!selectedGame) return;
    
    setIsDownloading(true);
    setDownloadStatus({ text: 'Initializing pipeline...', type: 'info' });
    
    try {
      // 1. Load active settings from Rust (to get current API key and Steam Path)
      setDownloadStatus({ text: 'Loading local configurations...', type: 'info' });
      const settings: any = await invoke('get_settings');
      
      const apiKeyToUse = selectedSource === 'hubcap' ? settings.hubcap_api_key : 'oureveryday_public';
      const steamPathToUse = settings.steam_path;
      
      if (selectedSource === 'hubcap' && (!apiKeyToUse || apiKeyToUse.trim() === '')) {
        setDownloadStatus({ text: 'Error: Please enter your Hubcap API Key in Settings first!', type: 'error' });
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
      const result: string = await invoke('trigger_hubcap_download', {
        appId: Number(selectedGame.appId),
        apiKey: apiKeyToUse,
        steamPath: steamPathToUse
      });
      
      setDownloadStatus({ text: result, type: 'success' });
      setIsDownloading(false);
      
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

  const handleDownloadOlder = () => {
    if (!selectedGame) return;
    alert(`Opening version picker dialog for: ${selectedGame.name} (${selectedGame.appId}) utilizing source ${selectedSource.toUpperCase()}.`);
    setSelectedGame(null); // close modal after trigger
  };

  return (
    <div className="store-view">
      {/* Upper header section */}
      <div className="store-header">
        <h1 className="store-title">Store</h1>
        <p className="store-subtitle">Browse, search and unlock game manifests using AetherDesk's built-in database.</p>
      </div>

      {/* Linea di separazione */}
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

      {/* Linea di separazione */}
      <div className="store-separator"></div>

      {/* 10 rows x 2 columns Grid */}
      <div className="store-grid">
        {isLoading ? (
          <div className="store-no-results">
            Loading results from Steam & Hubcap...
          </div>
        ) : pageGames.length > 0 ? (
          pageGames.map((game) => (
            <div key={game.id} className="store-game-card">
              {/* Dynamic absolute badge overlay in top-right corner, popping out */}
              {game.has_manifest && (
                <span className="badge-available">Available</span>
              )}

              {/* Cover Art Wrapper */}
              <div className="game-cover-wrapper">
                <img 
                  src={`https://cdn.cloudflare.steamstatic.com/steam/apps/${game.appId}/library_600x900.jpg`} 
                  alt={game.name}
                  className="game-cover-image"
                  onError={(e) => {
                    // Fallback placeholder if offline or cover image fails to load
                    (e.target as HTMLElement).style.display = 'none';
                  }}
                />
                <div className="game-cover-fallback">
                  <span>Æ</span>
                </div>
              </div>

              {/* Game Metadata and actions */}
              <div className="game-info-wrapper">
                <div className="game-details">
                  <h3 className="game-name" title={game.name}>{game.name}</h3>
                  <span className="game-appid">App ID: {game.appId}</span>
                </div>
                <button 
                  onClick={() => {
                    setSelectedGame(game);
                    setDownloadStatus({ text: '', type: 'info' });
                    setIsDownloading(false);
                    setSelectedSource(game.has_manifest ? 'hubcap' : 'oureveryday');
                  }} 
                  className="game-download-btn"
                >
                  Download
                </button>
              </div>
            </div>
          ))
        ) : (
          <div className="store-no-results">
            {hasSearched ? `No games found for "${activeQuery}"` : 'Enter a query above to search the Steam & Hubcap catalog.'}
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

            {/* Linea separatrice */}
            <div className="modal-separator"></div>

            {/* Modal Body Content */}
            <div className="modal-body">
              {/* Operation Feedback inside the Modal */}
              {downloadStatus.text && (
                <div className={`settings-alert ${downloadStatus.type}`} style={{ padding: '10px 15px', fontSize: '12px' }}>
                  {downloadStatus.text}
                </div>
              )}

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
                    onClick={() => setSelectedSource('oureveryday')}
                    className={`source-btn ${selectedSource === 'oureveryday' ? 'active' : ''}`}
                  >
                    OurEveryday
                  </button>
                  <button 
                    disabled={isDownloading}
                    onClick={() => setSelectedSource('local')}
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
                    Downloads the most recent manifest files and decryption keys directly into Steam. This allows Steam to download and install the latest official release natively at maximum connection speed.
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
    </div>
  );
};
