import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { InstalledGame, useLibraryGames } from '../hooks/useLibraryGames';
import { CrackModal } from './CrackModal';
import { AntivirusExclusionModal } from './AntivirusExclusionModal';

const MAX_VISIBLE_RESULTS = 5;

type HomeResource = 'onlinefix' | 'gcw' | 'csrinru';

interface SteamlessRunResult {
  success: boolean;
  cancelled: boolean;
  message: string;
  exePath?: string;
  backupPath?: string;
  stdoutTail: string;
  stderrTail: string;
}

type HomeStatus = { text: string; type: 'info' | 'success' | 'error' };

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
  const [activeResultIndex, setActiveResultIndex] = useState<number | null>(null);
  const [status, setStatus] = useState<HomeStatus>({ text: '', type: 'info' });
  const [isSteamlessRunning, setIsSteamlessRunning] = useState(false);
  // Game targeted by the "Apply Crack" popup; null means the popup is closed.
  const [crackTarget, setCrackTarget] = useState<{ name: string; appId: string } | null>(null);
  // Pending crack target waiting for the antivirus exclusion prompt to be dismissed.
  // When non-null, the antivirus modal is shown; once dismissed the CrackModal opens.
  const [pendingCrack, setPendingCrack] = useState<{ name: string; appId: string } | null>(null);
  const searchPanelRef = useRef<HTMLDivElement | null>(null);
  const resultRefs = useRef<Array<HTMLButtonElement | null>>([]);

  useEffect(() => {
    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (target && !searchPanelRef.current?.contains(target)) {
        setIsSearchOpen(false);
      }
    };

    document.addEventListener('mousedown', handlePointerDown);
    return () => document.removeEventListener('mousedown', handlePointerDown);
  }, []);

  const filteredGames = useMemo(() => {
    const sortedGames = [...games].sort((a, b) => a.name.localeCompare(b.name));

    if (!query.trim()) {
      return sortedGames;
    }

    return sortedGames.filter(game => Number.isFinite(fuzzyScore(query, game.name)));
  }, [games, query]);

  useEffect(() => {
    setActiveResultIndex(null);
  }, [query]);

  useEffect(() => {
    if (activeResultIndex !== null && activeResultIndex >= filteredGames.length) {
      setActiveResultIndex(filteredGames.length > 0 ? filteredGames.length - 1 : null);
    }
  }, [activeResultIndex, filteredGames.length]);

  useEffect(() => {
    if (!isSearchOpen || activeResultIndex === null) return;
    resultRefs.current[activeResultIndex]?.scrollIntoView({
      block: 'nearest',
    });
  }, [activeResultIndex, isSearchOpen]);


  const visibleRowCount = Math.min(MAX_VISIBLE_RESULTS, Math.max(filteredGames.length, 1));
  const hasSelectedGame = Boolean(selectedGame);

  const selectGame = (game: InstalledGame) => {
    setSelectedGame(game);
    setQuery(game.name);
    setIsSearchOpen(false);
    setActiveResultIndex(null);
    setStatus({ text: '', type: 'info' });
  };

  const selectActiveResult = () => {
    const game = activeResultIndex !== null
      ? filteredGames[activeResultIndex]
      : filteredGames.length === 1
        ? filteredGames[0]
        : null;
    if (game) {
      selectGame(game);
    }
  };

  const openResource = async (site: HomeResource) => {
    if (!selectedGame) {
      setStatus({ text: 'Select a Lua game first.', type: 'error' });
      return;
    }

    try {
      await invoke('open_home_resource', {
        site,
        gameName: selectedGame.name,
      });
      setStatus({ text: '', type: 'info' });
    } catch (err: any) {
      setStatus({ text: `Failed to open external resource: ${err}`, type: 'error' });
    }
  };


  const runSteamless = async () => {
    if (!selectedGame) {
      setStatus({ text: 'Select a Lua game first.', type: 'error' });
      return;
    }

    if (!selectedGame.installed || !selectedGame.gamePath) {
      setStatus({ text: 'Steamless requires the selected game to be installed locally.', type: 'error' });
      return;
    }

    setIsSteamlessRunning(true);
    setStatus({ text: 'Select the game executable to run Steamless...', type: 'info' });

    try {
      const result: SteamlessRunResult = await invoke('pick_and_run_steamless', {
        appId: selectedGame.id,
      });

      if (result.cancelled) {
        setStatus({ text: '', type: 'info' });
        return;
      }

      setStatus({
        text: result.message,
        type: result.success ? 'success' : 'error',
      });
    } catch (err: any) {
      setStatus({ text: `Steamless failed: ${err}`, type: 'error' });
    } finally {
      setIsSteamlessRunning(false);
    }
  };

  return (
    <div className="home-view">
      <h1 className="home-title">HOME</h1>

      <div className="home-search-panel" ref={searchPanelRef}>
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
              setActiveResultIndex(null);
              setStatus({ text: '', type: 'info' });
            }}
            onKeyDown={(event) => {
              if (event.key === 'ArrowDown') {
                event.preventDefault();
                setIsSearchOpen(true);
                setActiveResultIndex(prev => {
                  if (filteredGames.length === 0) return null;
                  return prev === null ? 0 : Math.min(prev + 1, filteredGames.length - 1);
                });
              } else if (event.key === 'ArrowUp') {
                event.preventDefault();
                setIsSearchOpen(true);
                setActiveResultIndex(prev => {
                  if (filteredGames.length === 0) return null;
                  return prev === null ? filteredGames.length - 1 : Math.max(prev - 1, 0);
                });
              } else if (event.key === 'Enter' || event.key === 'Delete') {
                // Empty search (no text typed, no game selected): dismiss the
                // whole suggestion list instead of selecting a game.
                if (!query.trim() && !selectedGame) {
                  event.preventDefault();
                  setIsSearchOpen(false);
                } else if (event.key === 'Enter' && isSearchOpen && filteredGames.length > 0) {
                  event.preventDefault();
                  selectActiveResult();
                }
              } else if (event.key === 'Escape') {
                setIsSearchOpen(false);
              }
            }}
          />

          {query && (
            <button
              type="button"
              className="home-search-clear"
              aria-label="Clear search"
              onClick={() => {
                setQuery('');
                setSelectedGame(null);
                setIsSearchOpen(false);
                setActiveResultIndex(null);
                setStatus({ text: '', type: 'info' });
              }}
            >
              &times;
            </button>
          )}

          {isSearchOpen && !isLoading && (
            <div
              className="home-search-results"
              style={{ maxHeight: `${visibleRowCount * 42}px` }}
            >
              {filteredGames.length > 0 ? (
                filteredGames.map((game, index) => (
                  <button
                    key={game.id}
                    type="button"
                    ref={(element) => { resultRefs.current[index] = element; }}
                    className={`home-search-result ${index === activeResultIndex ? 'active' : ''}`}
                    onMouseEnter={() => setActiveResultIndex(index)}
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
        <button
          className="game-action-btn"
          disabled={!hasSelectedGame}
          onClick={async () => {
            if (!selectedGame) return;
            const target = { name: selectedGame.name, appId: selectedGame.appId };

            // Check whether the antivirus exclusion has been confirmed.
            // If not, show the exclusion prompt first — but regardless of
            // the user's choice (confirm or X), always proceed to the
            // CrackModal so the user is never blocked.
            try {
              const done: boolean = await invoke('get_antivirus_exclusion_done');
              if (done) {
                setCrackTarget(target);
              } else {
                setPendingCrack(target);
              }
            } catch {
              // If the check fails, err on the side of showing the prompt.
              setPendingCrack(target);
            }
          }}
        >
          Apply Crack
        </button>
        <button
          className="game-action-btn"
          disabled={!hasSelectedGame}
          onClick={() => setStatus({ text: 'Enable DLC is not available yet.', type: 'info' })}
        >
          Enable DLC
        </button>
        <button
          className="game-action-btn"
          disabled={!hasSelectedGame || isSteamlessRunning}
          onClick={runSteamless}
        >
          {isSteamlessRunning ? 'Running...' : 'Steamless'}
        </button>
      </div>

      {status.text && <div className={`home-status ${status.type}`}>{status.text}</div>}

      {/* Antivirus exclusion prompt: shown before Apply Crack if never confirmed.
          Regardless of the user's choice, the CrackModal opens afterwards so the
          user is never blocked from applying the crack. */}
      {pendingCrack && (
        <AntivirusExclusionModal onDone={() => {
          const target = pendingCrack;
          setPendingCrack(null);
          setCrackTarget(target);
        }} />
      )}

      {crackTarget && (
        <CrackModal game={crackTarget} onClose={() => setCrackTarget(null)} />
      )}
    </div>
  );
};
