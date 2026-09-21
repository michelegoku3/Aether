import React, { useCallback, useEffect, useMemo, useState, memo } from 'react';
import { LuaManifestRow, prepareManifestRows } from '../modals/SpecificVersionModal';
import ChangeVersionModal from '../modals/ChangeVersionModal';
import { LibraryGameActionsModal } from '../modals/LibraryGameActionsModal';
import { GameInfoModal } from '../modals/GameInfoModal';
import { GameUpdatesModal } from '../modals/GameUpdatesModal';
import { WorkshopRepairModal } from '../modals/WorkshopRepairModal';
import { preloadGameCovers } from '../ui/GameCover';
import { GameCard, type GameCardAction } from '../ui/GameCard';
import { StatusAlert } from '../ui/StatusAlert';
import {
  ArrowUpThickIcon,
  CloseIcon,
  PlayIcon,
  RefreshIcon,
  WrenchIcon,
} from '../ui/icons';
import { useLibraryGames, InstalledGame } from '../hooks/useLibraryGames';
import { useLibraryInstallFilter } from '../hooks/useLibraryInstallFilter';
import { StatusType } from '../types/ui';
import { requireSteamPath } from '../hooks/useSettings';

interface LibraryViewProps {
  useAlternativeGameCards: boolean;
  alternativeCardsOpacity: number;
  alternativeCardsFade: number;
}

/** How many covers to eagerly preload when the visible list changes. One
 *  screenful of cards (≈ 12) plus one page over-scroll; enough to make
 *  scrolling feel instant without kicking off dozens of parallel image
 *  requests the user may never see (the previous value of 60 on every
 *  filtered-games change was flagged in the audit). */
const COVER_PRELOAD_WINDOW = 24;
export const LibraryView = memo(function LibraryView({
  useAlternativeGameCards,
  alternativeCardsOpacity,
  alternativeCardsFade,
}: LibraryViewProps) {
  const { games, isLoading, isRefreshing, status, setStatus, loadInstalledGames, queryGameState } =
    useLibraryGames();
  const [searchQuery, setSearchQuery] = useState('');

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
  // Gate del bulk editor degli aggiornamenti; false = popup chiuso. Il
  // percorso Steam non è più un dato della UI: lo risolvono i comandi.
  const [showUpdatesModal, setShowUpdatesModal] = useState(false);
  // Workshop manifest repair (scan appworkshop ACF + stage missing manifests).
  const [showWorkshopRepair, setShowWorkshopRepair] = useState(false);


  const showStatus = (text: string, type: StatusType) => {
    setStatus({ text, type });
  };

  useEffect(() => {
    if (filteredGames.length === 0) return;
    // When the tab/document is hidden we skip eager preloading: the GameCard
    // component itself will resolve its own cover when mounted, so preloading
    // unseen cards off-screen is wasted work.
    if (document.visibilityState !== 'visible') return;
    preloadGameCovers(
      filteredGames.slice(0, COVER_PRELOAD_WINDOW).map((game) => ({ appId: game.appId, imageUrl: game.imageUrl })),
      COVER_PRELOAD_WINDOW,
    );
  }, [filteredGames]);

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

  const handleOpenUpdatesModal = async () => {
    try {
      // Pre-volo volutamente mantenuto: senza percorso configurato il popup si
      // aprirebbe per fallire su OGNI gioco (un errore per riga). Costa una
      // sola lettura impostazioni per click, non una per gioco come prima.
      await requireSteamPath();
      setShowUpdatesModal(true);
    } catch (err: any) {
      showStatus(`Unable to manage game updates: ${err}`, 'error');
    }
  };

  // useCallback: è la prop `onFixPins` di OGNI card della griglia. Ricrearla a
  // ogni render annullerebbe `memo(GameCard)`, che è proprio ciò che evita di
  // ri-renderizzare N card a ogni keystroke della ricerca o a ogni toast.
  // I setter di stato sono stabili per garanzia React, quindi l'unica
  // dipendenza reale è `queryGameState` (già useCallback nel provider).
  const handleOpenVersionEditor = useCallback(async (game: InstalledGame) => {
    setStatus({ text: '', type: 'info' });
    const appId = Number(game.appId);

    // The shared provider cache covers the "close and re-open the editor"
    // case (and the StrictMode double mount) without a second, shorter-lived
    // snapshot to keep coherent: every completed library scan invalidates it.
    try {
      const rows = await queryGameState<LuaManifestRow[]>(appId, 'get_installed_lua_manifest_rows');
      setManifestRows(prepareManifestRows(rows));
      setVersionGame(game);
    } catch (err: any) {
      setStatus({
        text: `Unable to open version editor for ${game.name}: ${err}`,
        type: 'error',
      });
    }
  }, [queryGameState]);

  // Le azioni della card non dipendono dal singolo gioco (ricevono `game` dal
  // click) e usano solo setter stabili: l'array si costruisce UNA volta.
  // Ricrearlo per ogni card a ogni render è ciò che rendeva inutile
  // `memo(GameCard)`.
  const cardActions = useMemo<Array<GameCardAction<InstalledGame>>>(() => [
    { label: 'Modify', variant: 'primary', onClick: setActionGame },
    { label: 'Info', variant: 'secondary', onClick: setInfoGame },
  ], []);

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

        <div className="library-search-panel">
          <div className="home-search-wrapper library-search-wrapper">
            <input
              type="text"
              className="home-search-input library-search-input"
              value={searchQuery}
              placeholder={isLoading ? 'Loading Lua games...' : 'Search a Lua game...'}
              disabled={isLoading}
              onChange={(event) => setSearchQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Escape') {
                  setSearchQuery('');
                }
              }}
            />

            {searchQuery && (
              <button
                type="button"
                className="home-search-clear"
                aria-label="Clear search"
                onClick={() => setSearchQuery('')}
              >
                &times;
              </button>
            )}
          </div>
        </div>

        <div className="library-toolbar-actions">
          <button
            type="button"
            className="library-icon-btn"
            onClick={() => void handleOpenUpdatesModal()}
            disabled={isLoading}
            title="Manage game updates (block/unblock all)"
            aria-label="Manage game updates"
          >
            <ArrowUpThickIcon />
          </button>
          <button
            type="button"
            className="library-icon-btn"
            onClick={() => setShowWorkshopRepair(true)}
            disabled={isLoading}
            title="Repair Steam Workshop manifests"
            aria-label="Repair Steam Workshop manifests"
          >
            <WrenchIcon />
          </button>
          <button
            type="button"
            className="library-icon-btn"
            onClick={loadInstalledGames}
            disabled={isLoading || isRefreshing}
            title={isRefreshing ? 'Refreshing library…' : 'Refresh library'}
            aria-label="Refresh library"
            aria-busy={isRefreshing}
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
              // The badge is the shortest path to the fix: it opens the same
              // editor as Modify → Change Version, where the bad line is red.
              onFixPins={handleOpenVersionEditor}
              actions={cardActions}
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

      {showUpdatesModal && (
        <GameUpdatesModal
          games={games}
          onStatus={showStatus}
          onRefresh={loadInstalledGames}
          onClose={() => setShowUpdatesModal(false)}
        />
      )}

      {showWorkshopRepair && (
        <WorkshopRepairModal
          onStatus={showStatus}
          onClose={() => setShowWorkshopRepair(false)}
        />
      )}
    </div>
  );
});
