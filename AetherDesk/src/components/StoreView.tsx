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
      alert(`Errore di ricerca: ${err}`);
      setIsLoading(false);
    }
  };

  const handleDownloadSteam = async () => {
    if (!selectedGame) return;
    
    setIsDownloading(true);
    setDownloadStatus({ text: 'Inizializzazione della pipeline...', type: 'info' });
    
    try {
      // 1. Load active settings from Rust (to get current API key and Steam Path)
      setDownloadStatus({ text: 'Caricamento delle impostazioni locali...', type: 'info' });
      const settings: any = await invoke('get_settings');
      
      const apiKeyToUse = selectedSource === 'hubcap' ? settings.hubcap_api_key : 'oureveryday_public';
      const steamPathToUse = settings.steam_path;
      
      if (selectedSource === 'hubcap' && (!apiKeyToUse || apiKeyToUse.trim() === '')) {
        setDownloadStatus({ text: 'Errore: Inserisci prima la tua chiave API di Hubcap nelle Impostazioni!', type: 'error' });
        setIsDownloading(false);
        return;
      }
      if (!steamPathToUse || steamPathToUse.trim() === '') {
        setDownloadStatus({ text: 'Errore: Specifica prima il percorso di Steam nelle Impostazioni!', type: 'error' });
        setIsDownloading(false);
        return;
      }

      // 2. Invoke the decoupled, professional Rust download orchestrator!
      setDownloadStatus({ text: `Connessione alla fonte ${selectedSource.toUpperCase()}...`, type: 'info' });
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
      setDownloadStatus({ text: `Download fallito: ${err}`, type: 'error' });
      setIsDownloading(false);
    }
  };

  const handleDownloadOlder = () => {
    if (!selectedGame) return;
    alert(`Apertura selettore versione precedente per: ${selectedGame.name} (${selectedGame.appId}) con sorgente ${selectedSource.toUpperCase()}.`);
    setSelectedGame(null); // close modal after trigger
  };

  return (
    <div className="store-view">
      {/* Upper header section */}
      <div className="store-header">
        <h1 className="store-title">Store</h1>
        <p className="store-subtitle">Sfoglia, ricerca e sblocca i manifesti dei tuoi giochi tramite il database integrato di AetherDesk.</p>
      </div>

      {/* Linea di separazione */}
      <div className="store-separator"></div>

      {/* Search Input Area */}
      <form onSubmit={handleSearch} className="store-search-form">
        <input 
          type="text" 
          placeholder="Cerca gioco per nome o App ID su Steam..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          className="store-search-input"
        />
        <button type="submit" className="store-search-btn" disabled={isLoading}>
          {isLoading ? 'Searching...' : 'Search'}
        </button>
      </form>

      {/* Linea di separazione */}
      <div className="store-separator"></div>

      {/* 10 rows x 2 columns Grid */}
      <div className="store-grid">
        {isLoading ? (
          <div className="store-no-results">
            Caricamento risultati in corso da Steam & Hubcap...
          </div>
        ) : pageGames.length > 0 ? (
          pageGames.map((game) => (
            <div key={game.id} className="store-game-card">
              {/* Dynamic absolute badge overlay in top-right corner, popping out */}
              {game.has_manifest && (
                <span className="badge-available">Disponibile</span>
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
            {hasSearched ? `Nessun gioco trovato per "${activeQuery}"` : 'Inserisci una query sopra per cercare nel catalogo Steam & Hubcap.'}
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
            Pagina {page} di {totalPages}
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
                <span className="source-label">Fonte:</span>
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

              {/* Action 1: Download through Steam Button */}
              <button 
                onClick={handleDownloadSteam}
                className="big-action-btn"
                disabled={isDownloading}
                style={{ opacity: isDownloading ? 0.5 : 1 }}
              >
                <div className="action-icon">⚡</div>
                <div className="action-info">
                  <span className="action-title">Download through Steam</span>
                  <span className="action-desc">
                    Aggiunge il gioco alla libreria di Steam. Apri Steam per scaricare. Scarica i manifesti e le chiavi in modo che Steam installi il gioco nativamente.
                  </span>
                </div>
              </button>

              {/* Action 2: Download Older Version Button */}
              <button 
                onClick={handleDownloadOlder}
                className="big-action-btn"
                disabled={isDownloading}
                style={{ opacity: isDownloading ? 0.5 : 1 }}
              >
                <div className="action-icon">📦</div>
                <div className="action-info">
                  <span className="action-title">Download Older Version</span>
                  <span className="action-desc">
                    Seleziona e scarica una versione precedente specifica tramite DDMod o Steam (Nativo).
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
