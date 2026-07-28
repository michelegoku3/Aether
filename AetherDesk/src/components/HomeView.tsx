import { useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { InstalledGame, useLibraryGames } from '../hooks/useLibraryGames';

const MAX_VISIBLE_RESULTS = 5;

type HomeResource = 'onlinefix' | 'gcw' | 'csrinru';

const normalizeSearchText = (value: string) =>
  value
    .toLowerCase()
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '')
    .replace(/[^a-z0-9]+/g, ' ')
    .trim();

const fuzzyScore = (query: string, candidate: string) => {
  const q = normalizeSearchText(query);
  const c = normalizeSearchText(candidate);

  if (!q) return 0;
  if (c === q) return 0;
  if (c.startsWith(q)) return 1 + c.length - q.length;
  if (c.includes(q)) return 100 + (c.indexOf(q) * 2) + c.length - q.length;

  let lastIndex = -1;
  let score = 500;
  for (const char of q) {
    const index = c.indexOf(char, lastIndex + 1);
    if (index === -1) return Number.POSITIVE_INFINITY;
    score += index - lastIndex;
    lastIndex = index;
  }

  return score + c.length - q.length;
};

export const HomeView = () => {
  const { games, isLoading } = useLibraryGames();
  const [query, setQuery] = useState('');
  const [selectedGame, setSelectedGame] = useState<InstalledGame | null>(null);
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const [status, setStatus] = useState('');

  const filteredGames = useMemo(() => {
    const sortedGames = [...games].sort((a, b) => a.name.localeCompare(b.name));

    if (!query.trim()) {
      return sortedGames;
    }

    return sortedGames.filter(game => Number.isFinite(fuzzyScore(query, game.name)));
  }, [games, query]);

  const visibleRowCount = Math.min(MAX_VISIBLE_RESULTS, Math.max(filteredGames.length, 1));
  const hasSelectedGame = Boolean(selectedGame);

  const selectGame = (game: InstalledGame) => {
    setSelectedGame(game);
    setQuery(game.name);
    setIsSearchOpen(false);
    setStatus('');
  };

  const openResource = async (site: HomeResource) => {
    if (!selectedGame) {
      setStatus('Select a Lua game first.');
      return;
    }

    try {
      await invoke('open_home_resource', {
        site,
        gameName: selectedGame.name,
      });
      setStatus('');
    } catch (err: any) {
      setStatus(`Failed to open external resource: ${err}`);
    }
  };

  return (
    <div className="home-view">
      <h1 className="home-title">HOME</h1>

      <div className="home-search-panel">
        <div className="home-search-wrapper">
          <input
            className="home-search-input"
            value={query}
            placeholder={isLoading ? 'Loading Lua games...' : 'Search a Lua game...'}
            disabled={isLoading}
            onFocus={() => setIsSearchOpen(true)}
            onChange={(event) => {
              setQuery(event.target.value);
              setSelectedGame(null);
              setIsSearchOpen(true);
              setStatus('');
            }}
            onKeyDown={(event) => {
              if (event.key === 'Escape') {
                setIsSearchOpen(false);
              }
            }}
          />

          {isSearchOpen && !isLoading && (
            <div
              className="home-search-results"
              style={{ maxHeight: `${visibleRowCount * 42}px` }}
            >
              {filteredGames.length > 0 ? (
                filteredGames.map(game => (
                  <button
                    key={game.id}
                    type="button"
                    className="home-search-result"
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => selectGame(game)}
                  >
                    <span className="home-search-result-name">{game.name}</span>
                    <span className="home-search-result-appid">{game.appId}</span>
                  </button>
                ))
              ) : (
                <div className="home-search-empty">No Lua games found.</div>
              )}
            </div>
          )}
        </div>

        {selectedGame && (
          <div className="home-selected-game">
            Selected: <strong>{selectedGame.name}</strong>
          </div>
        )}
      </div>

      <div className="home-action-grid">
        <button
          className="game-action-btn"
          disabled={!hasSelectedGame}
          onClick={() => openResource('onlinefix')}
        >
          OnlineFix
        </button>
        <button
          className="game-action-btn"
          disabled={!hasSelectedGame}
          onClick={() => openResource('gcw')}
        >
          GCW
        </button>
        <button
          className="game-action-btn"
          disabled={!hasSelectedGame}
          onClick={() => openResource('csrinru')}
        >
          CSRINRU
        </button>
        <button className="game-action-btn" disabled>
          Steamless
        </button>
      </div>

      {status && <div className="home-status">{status}</div>}
    </div>
  );
};
