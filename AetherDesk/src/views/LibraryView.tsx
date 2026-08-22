import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { LuaManifestRow } from '../modals/SpecificVersionModal';
import ChangeVersionModal from '../modals/ChangeVersionModal';
import { LibraryGameActionsModal } from '../modals/LibraryGameActionsModal';
import { GameInfoModal } from '../modals/GameInfoModal';
import { preloadGameCovers } from '../ui/GameCover';
import { GameCard } from '../ui/GameCard';
import { StatusAlert } from '../ui/StatusAlert';
import { ArrowUpThickIcon, CloseIcon, PlayIcon, RefreshIcon } from '../ui/icons';
import { SearchSuggest, moveSuggestIndex, SearchSuggestItem } from '../ui/SearchSuggest';
import { useLibraryGames, InstalledGame } from '../hooks/useLibraryGames';
import { useLibraryInstallFilter } from '../hooks/useLibraryInstallFilter';
import { StatusType } from '../types/ui';
import { requireSteamPath } from '../hooks/useSettings';

const MAX_VISIBLE_RESULTS = 3;
const AUTO_DISMISS_SUGGEST_DELAY_MS = 1000;

interface LibraryViewProps {
  useAlternativeGameCards: boolean;
  alternativeCardsOpacity: number;
  alternativeCardsFade: number;
}

export const LibraryView = ({
  useAlternativeGameCards,
  alternativeCardsOpacity,
  alternativeCardsFade,
}: LibraryViewProps) => {
  const { games, isLoading, status, setStatus, loadInstalledGames } = useLibraryGames();
  const [searchQuery, setSearchQuery] = useState('');
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const [activeResultIndex, setActiveResultIndex] = useState<number | null>(null);
  const searchPanelRef = useRef<HTMLDivElement | null>(null);
  const suggestTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const {
    filter,
    ready: filterReady,
    filteredGames,
    cycleFilter,
    filterTitle,
    countLabel,
    emptyMessage,
    filterButtonClass,
  } = useLibraryInstallFilter(games, isLoading, searchQuery);

  const [actionGame, setActionGame] = useState<InstalledGame | null>(null);
  const [infoGame, setInfoGame] = useState<InstalledGame | null>(null);
  const [versionGame, setVersionGame] = useState<InstalledGame | null>(null);
  const [manifestRows, setManifestRows] = useState<LuaManifestRow[]>([]);

  const showStatus = (text: string, type: StatusType) => {
    setStatus({ text, type });
  };

  const clearSuggestTimer = () => {
    if (suggestTimerRef.current) {
      clearTimeout(suggestTimerRef.current);
      suggestTimerRef.current = null;
    }
  };

  const scheduleSuggestDismiss = () => {
    clearSuggestTimer();
    suggestTimerRef.current = setTimeout(() => {
      setIsSearchOpen(false);
    }, AUTO_DISMISS_SUGGEST_DELAY_MS);
  };

  useEffect(() => {
    return () => {
      clearSuggestTimer();
    };
  }, []);

  useEffect(() => {
    if (filteredGames.length > 0) {
      preloadGameCovers(
        filteredGames.slice(0, 60).map((game) => ({ appId: game.appId, imageUrl: game.imageUrl })),
        60,
      );
    }
  }, [filteredGames]);

  useEffect(() => {
    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (target && !searchPanelRef.current?.contains(target)) {
        clearSuggestTimer();
        setIsSearchOpen(false);
      }
    };

    document.addEventListener('mousedown', handlePointerDown);
    return () => document.removeEventListener('mousedown', handlePointerDown);
  }, []);

  useEffect(() => {
    setActiveResultIndex(null);
  }, [searchQuery]);

  useEffect(() => {
    if (activeResultIndex !== null && activeResultIndex >= filteredGames.length) {
      setActiveResultIndex(filteredGames.length > 0 ? filteredGames.length - 1 : null);
    }
  }, [activeResultIndex, filteredGames.length]);

  const selectActiveResult = () => {
    clearSuggestTimer();
    const game = activeResultIndex !== null
      ? filteredGames[activeResultIndex]
      : filteredGames.length === 1
        ? filteredGames[0]
        : null;
    if (game) {
      setSearchQuery(game.name);
      setIsSearchOpen(false);
      setActiveResultIndex(null);
    }
  };

  const handleSelectSuggest = (item: SearchSuggestItem) => {
    clearSuggestTimer();
    setSearchQuery(item.name);
    setIsSearchOpen(false);
    setActiveResultIndex(null);
  };

  const handleClearSearch = () => {
    clearSuggestTimer();
    setSearchQuery('');
    setIsSearchOpen(false);
    setActiveResultIndex(null);
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    const hasQuery = Boolean(searchQuery.trim());

    if (event.key === 'ArrowDown') {
      if (hasQuery && filteredGames.length > 0) {
        event.preventDefault();
        setIsSearchOpen(true);
        setActiveResultIndex((prev) => moveSuggestIndex(prev, filteredGames.length, 1));
        scheduleSuggestDismiss();
      }
    } else if (event.key === 'ArrowUp') {
      if (hasQuery && filteredGames.length > 0) {
        event.preventDefault();
        setIsSearchOpen(true);
        setActiveResultIndex((prev) => moveSuggestIndex(prev, filteredGames.length, -1));
        scheduleSuggestDismiss();
      }
    } else if (event.key === 'Enter') {
      event.preventDefault();
      if (isSearchOpen && (activeResultIndex !== null || filteredGames.length === 1)) {
        selectActiveResult();
      } else {
        clearSuggestTimer();
        setIsSearchOpen(false);
      }
    } else if (event.key === 'Escape') {
      clearSuggestTimer();
      setIsSearchOpen(false);
    }
  };

  const handleCycleFilter = async () => {
    try {
      await cycleFilter();
    } catch (err: any) {
      setStatus({
        text: `Unable to save library filter: ${err}`,
        type: 'error',
      });
    }
  };

  const handleOpenVersionEditor = async (game: InstalledGame) => {
    setStatus({ text: '', type: 'info' });

    try {
      const steamPath = await requireSteamPath();
      const rows: LuaManifestRow[] = await invoke('get_installed_lua_manifest_rows', {
        appId: Number(game.appId),
        steamPath,
      });

      setManifestRows((rows || []).map((row) => ({ ...row, manifestInput: '' })));
      setVersionGame(game);
    } catch (err: any) {
      setStatus({
        text: `Unable to open version editor for ${game.name}: ${err}`,
        type: 'error',
      });
    }
  };

  const isSuggestVisible = isSearchOpen && !isLoading && Boolean(searchQuery.trim());

  return (
    <div className="store-view">
      <div className="store-header">
        <h1 className="store-title">Library</h1>
        <p className="store-subtitle">
          Manage every game Lua installed in Steam's stplug-in folder, with install status detected
          from Steam appmanifest ACF files.
        </p>
      </div>

      <div className="store-separator"></div>

      <StatusAlert status={status} />

      <div className="library-toolbar">
        <span className="library-count">{countLabel}</span>

        <div
          className="library-search-panel"
          ref={searchPanelRef}
          onMouseEnter={clearSuggestTimer}
          onMouseLeave={() => {
            if (isSearchOpen && searchQuery.trim()) {
              scheduleSuggestDismiss();
            }
          }}
        >
          <div className="home-search-wrapper library-search-wrapper">
            <input
              type="text"
              className="home-search-input library-search-input"
              value={searchQuery}
              placeholder={isLoading ? 'Loading Lua games...' : 'Search a Lua game...'}
              disabled={isLoading}
              onChange={(event) => {
                const val = event.target.value;
                setSearchQuery(val);
                setActiveResultIndex(null);
                if (val.trim()) {
                  setIsSearchOpen(true);
                  scheduleSuggestDismiss();
                } else {
                  clearSuggestTimer();
                  setIsSearchOpen(false);
                }
              }}
              onKeyDown={handleKeyDown}
            />

            {searchQuery && (
              <button
                type="button"
                className="home-search-clear"
                aria-label="Clear search"
                onClick={handleClearSearch}
              >
                &times;
              </button>
            )}

            <SearchSuggest
              open={isSuggestVisible}
              items={filteredGames}
              emptyText="No Lua games found."
              activeIndex={activeResultIndex}
              maxVisible={MAX_VISIBLE_RESULTS}
              onHoverIndex={setActiveResultIndex}
              onSelect={handleSelectSuggest}
            />
          </div>
        </div>

        <div className="library-toolbar-actions">
          <button
            type="button"
            className="library-icon-btn"
            disabled={true}
            tabIndex={-1}
            title="Update"
            aria-label="Update"
          >
            <ArrowUpThickIcon />
          </button>
          <button
            type="button"
            className="library-icon-btn"
            onClick={loadInstalledGames}
            disabled={isLoading}
            title="Refresh library"
            aria-label="Refresh library"
          >
            <RefreshIcon />
          </button>
          <button
            type="button"
            className={`library-icon-btn ${filterButtonClass}`.trim()}
            onClick={handleCycleFilter}
            disabled={isLoading || !filterReady}
            title={filterTitle}
            aria-label={filterTitle}
          >
            {filter === 'not_installed' ? <CloseIcon /> : <PlayIcon />}
          </button>
        </div>
      </div>

      <div className="store-separator"></div>

      <div
        className={useAlternativeGameCards ? 'store-grid alt-card-grid' : 'store-grid'}
        style={
          useAlternativeGameCards
            ? ({
                '--alt-card-opacity': Math.max(0, Math.min(100, alternativeCardsOpacity)),
                '--alt-card-fade': Math.max(0, Math.min(100, alternativeCardsFade)),
              } as React.CSSProperties)
            : undefined
        }
      >
        {isLoading ? (
          <div className="store-no-results">Scanning Steam appmanifest files...</div>
        ) : filteredGames.length > 0 ? (
          filteredGames.map((game) => (
            <GameCard
              key={game.id}
              game={game}
              cardVariant={useAlternativeGameCards ? 'backdrop' : 'classic'}
              actions={[
                {
                  label: 'Modify',
                  variant: 'primary',
                  onClick: setActionGame,
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
          <div className="store-no-results">{emptyMessage}</div>
        )}
      </div>

      {infoGame && (
        <GameInfoModal
          appId={Number(infoGame.appId)}
          fallbackName={infoGame.name}
          fallbackImageUrl={infoGame.imageUrl}
          onClose={() => setInfoGame(null)}
        />
      )}

      {actionGame && !versionGame && (
        <LibraryGameActionsModal
          game={actionGame}
          isProcessing={false}
          onClose={() => setActionGame(null)}
          onStatus={showStatus}
          onRefresh={loadInstalledGames}
          onOpenVersionEditor={handleOpenVersionEditor}
        />
      )}

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
    </div>
  );
};
