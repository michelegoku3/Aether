import React, { useState } from 'react';

export interface Game {
  id: number;
  name: string;
  appId: string;
}

// Mock database of 30 games to demonstrate professional pagination and searching
const MOCK_GAMES: Game[] = [
  { id: 1, name: "Ready or Not", appId: "1144200" },
  { id: 2, name: "Cyberpunk 2077", appId: "1091500" },
  { id: 3, name: "Elden Ring", appId: "1245620" },
  { id: 4, name: "Black Myth: Wukong", appId: "2358720" },
  { id: 5, name: "Helldivers 2", appId: "553850" },
  { id: 6, name: "Baldur's Gate 3", appId: "1086940" },
  { id: 7, name: "Grand Theft Auto V", appId: "271590" },
  { id: 8, name: "Hogwarts Legacy", appId: "990080" },
  { id: 9, name: "Sifu", appId: "2138710" },
  { id: 10, name: "Red Dead Redemption 2", appId: "1174180" },
  { id: 11, name: "Dying Light 2", appId: "534380" },
  { id: 12, name: "The Witcher 3: Wild Hunt", appId: "292030" },
  { id: 13, name: "Monster Hunter: World", appId: "582010" },
  { id: 14, name: "Sekiro: Shadows Die Twice", appId: "814380" },
  { id: 15, name: "Forza Horizon 5", appId: "1551360" },
  { id: 16, name: "Resident Evil 4 (Remake)", appId: "2050650" },
  { id: 17, name: "Street Fighter 6", appId: "1364780" },
  { id: 18, name: "Armored Core VI", appId: "1888140" },
  { id: 19, name: "Dead Space (Remake)", appId: "1693980" },
  { id: 20, name: "Lies of P", appId: "1627720" },
  { id: 21, name: "Tekken 8", appId: "1778820" },
  { id: 22, name: "Dragon's Dogma 2", appId: "2054970" },
  { id: 23, name: "Doom Eternal", appId: "782330" },
  { id: 24, name: "Horizon Zero Dawn", appId: "1151640" },
  { id: 25, name: "Death Stranding", appId: "1190460" },
  { id: 26, name: "Ghost of Tsushima", appId: "2215430" },
  { id: 27, name: "God of War", appId: "1593500" },
  { id: 28, name: "Marvel's Spider-Man Remastered", appId: "1817070" },
  { id: 29, name: "Control", appId: "870780" },
  { id: 30, name: "Cyberpunk 2077: Phantom Liberty", appId: "1868340" }
];

export const StoreView = () => {
  const [searchQuery, setSearchQuery] = useState('');
  const [activeQuery, setActiveQuery] = useState('');
  const [page, setPage] = useState(1);
  const itemsPerPage = 20; // 10 rows * 2 columns = 20 items per page

  // Active game selected for download modal, null means modal is closed
  const [selectedGame, setSelectedGame] = useState<Game | null>(null);
  
  // Selected key/manifest source state ('hubcap', 'oureveryday', 'local')
  const [selectedSource, setSelectedSource] = useState<'hubcap' | 'oureveryday' | 'local'>('oureveryday');

  // Filter games based on search query
  const filteredGames = MOCK_GAMES.filter(game => 
    game.name.toLowerCase().includes(activeQuery.toLowerCase()) || 
    game.appId.includes(activeQuery)
  );

  // Pagination calculation
  const totalPages = Math.ceil(filteredGames.length / itemsPerPage) || 1;
  const startIndex = (page - 1) * itemsPerPage;
  const pageGames = filteredGames.slice(startIndex, startIndex + itemsPerPage);

  const handleSearch = (e: React.FormEvent) => {
    e.preventDefault();
    setActiveQuery(searchQuery);
    setPage(1); // reset to first page on new search
  };

  const handleDownloadSteam = () => {
    if (!selectedGame) return;
    alert(`Esecuzione download tramite Steam per: ${selectedGame.name} (${selectedGame.appId}) con sorgente ${selectedSource.toUpperCase()}.`);
    setSelectedGame(null); // close modal after trigger
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
          placeholder="Cerca gioco per nome o App ID..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          className="store-search-input"
        />
        <button type="submit" className="store-search-btn">
          Search
        </button>
      </form>

      {/* Linea di separazione */}
      <div className="store-separator"></div>

      {/* 10 rows x 2 columns Grid */}
      <div className="store-grid">
        {pageGames.length > 0 ? (
          pageGames.map((game) => (
            <div key={game.id} className="store-game-card">
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
                  onClick={() => setSelectedGame(game)} // open modal on click
                  className="game-download-btn"
                >
                  Download
                </button>
              </div>
            </div>
          ))
        ) : (
          <div className="store-no-results">
            Nessun gioco trovato per "{activeQuery}"
          </div>
        )}
      </div>

      {/* Pagination controls below the grid */}
      {totalPages > 1 && (
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
                onClick={() => setSelectedGame(null)} 
                className="modal-close-btn"
              >
                &times;
              </button>
            </div>

            {/* Linea separatrice */}
            <div className="modal-separator"></div>

            {/* Modal Body Content */}
            <div className="modal-body">
              {/* Dark Source Box Panel (Parte di colore piu scuro) */}
              <div className="source-box">
                <span className="source-label">Fonte:</span>
                <div className="source-buttons-row">
                  <button 
                    onClick={() => setSelectedSource('hubcap')}
                    className={`source-btn ${selectedSource === 'hubcap' ? 'active' : ''}`}
                  >
                    Hubcap
                  </button>
                  <button 
                    onClick={() => setSelectedSource('oureveryday')}
                    className={`source-btn ${selectedSource === 'oureveryday' ? 'active' : ''}`}
                  >
                    OurEveryday
                  </button>
                  <button 
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
