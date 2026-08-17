import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { InstalledGame, useLibraryGames } from '../hooks/useLibraryGames';
import { CrackModal, CrackTargetGame } from '../modals/CrackModal';
import { FindCrackModal } from '../modals/FindCrackModal';
import { SavedCrackModal } from '../modals/SavedCrackModal';
import { SearchSuggest, moveSuggestIndex } from '../ui/SearchSuggest';

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
  // Drop-zone Apply Crack modal target.
  const [crackTarget, setCrackTarget] = useState<CrackTargetGame | null>(null);
  // Saved-crack reuse prompt (backup/<appId>/crack already populated).
  const [savedCrackTarget, setSavedCrackTarget] = useState<CrackTargetGame | null>(null);
  const [showFindCrack, setShowFindCrack] = useState(false);
  // Waiting for antivirus exclusion prompt; after dismiss we continue the crack flow.
  const searchPanelRef = useRef<HTMLDivElement | null>(null);

  /**
   * Entry point after antivirus gate: probe for a saved crack backup, then
   * either prompt to reuse it or open the normal drop-zone modal.
   */
  const beginCrackFlow = async (target: CrackTargetGame) => {
    try {
      const hasSaved: boolean = await invoke('has_saved_crack', {
        appId: Number(target.appId),
      });
      if (hasSaved) {
        setSavedCrackTarget(target);
      } else {
        setCrackTarget(target);
      }
    } catch {
      // Probe failure must not block Apply Crack.
      setCrackTarget(target);
    }
  };

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
                setActiveResultIndex((prev) => moveSuggestIndex(prev, filteredGames.length, 1));
              } else if (event.key === 'ArrowUp') {
                event.preventDefault();
                setIsSearchOpen(true);
                setActiveResultIndex((prev) => moveSuggestIndex(prev, filteredGames.length, -1));
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

          <SearchSuggest
            open={isSearchOpen && !isLoading}
            items={filteredGames}
            emptyText="No Lua games found."
            activeIndex={activeResultIndex}
            maxVisible={MAX_VISIBLE_RESULTS}
            onHoverIndex={setActiveResultIndex}
            onSelect={(item) => {
              const game = filteredGames.find((candidate) => candidate.id === item.id);
              if (game) selectGame(game);
            }}
          />
        </div>

      </div>

      <div className="home-action-grid">
        <button
          className="game-action-btn"
          disabled={!hasSelectedGame}
          onClick={() => {
            if (!hasSelectedGame) {
              setStatus({ text: 'Select a Lua game first.', type: 'error' });
              return;
            }
            setShowFindCrack(true);
          }}
        >
          Find Crack
        </button>
        <button
          className="game-action-btn"
          disabled={!hasSelectedGame}
          onClick={async () => {
            if (!selectedGame) return;
            const target: CrackTargetGame = {
              name: selectedGame.name,
              appId: selectedGame.appId,
            };
            await beginCrackFlow(target);
          }}
        >
          Apply Crack
        </button>
        <button
          className="game-action-btn"
          disabled={true}
          title="Enable DLC is not available yet"
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

      {showFindCrack && (
        <FindCrackModal
          onClose={() => setShowFindCrack(false)}
          onSelect={(site) => {
            setShowFindCrack(false);
            openResource(site);
          }}
        />
      )}

      {savedCrackTarget && (
        <SavedCrackModal
          game={savedCrackTarget}
          onReapplied={(message) => {
            setSavedCrackTarget(null);
            setStatus({ text: message, type: 'success' });
          }}
          onDeclined={(cleanupMessage) => {
            const target = savedCrackTarget;
            setSavedCrackTarget(null);
            if (cleanupMessage) {
              setStatus({ text: cleanupMessage, type: 'info' });
            }
            setCrackTarget(target);
          }}
          onCancel={() => setSavedCrackTarget(null)}
        />
      )}

      {crackTarget && (
        <CrackModal game={crackTarget} onClose={() => setCrackTarget(null)} />
      )}
    </div>
  );
};
