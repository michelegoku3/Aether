import React, { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { SpecificVersionModal, LuaManifestRow } from '../modals/SpecificVersionModal';
import { LibraryGameActionsModal } from '../modals/LibraryGameActionsModal';
import { GameInfoModal } from '../modals/GameInfoModal';
import { preloadGameCovers } from '../ui/GameCover';
import { GameCard } from '../ui/GameCard';
import { StatusAlert } from '../ui/StatusAlert';
import { useLibraryGames, InstalledGame } from '../hooks/useLibraryGames';
import { StatusType } from '../types/ui';
import { getSettings, requireSteamPath } from '../hooks/useSettings';

type LibraryInstallFilter = 'all' | 'installed' | 'not_installed';

const FILTER_CYCLE: LibraryInstallFilter[] = ['all', 'installed', 'not_installed'];

const normalizeLibraryInstallFilter = (value: unknown): LibraryInstallFilter => {
  const raw = String(value ?? '').trim().toLowerCase();
  if (raw === 'installed') return 'installed';
  if (raw === 'not_installed' || raw === 'not-installed' || raw === 'uninstalled') {
    return 'not_installed';
  }
  return 'all';
};

interface LibraryViewProps {
  useAlternativeGameCards: boolean;
  alternativeCardsOpacity: number;
  alternativeCardsFade: number;
}

const PlayIcon = () => (
  <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
    <path d="M8 5.14v13.72c0 .79.87 1.27 1.54.84l10.14-6.86a1 1 0 0 0 0-1.68L9.54 4.3A1 1 0 0 0 8 5.14z" />
  </svg>
);

const CloseIcon = () => (
  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path
      d="M6 6l12 12M18 6L6 18"
      stroke="currentColor"
      strokeWidth="2.4"
      strokeLinecap="round"
    />
  </svg>
);

const RefreshIcon = () => (
  <svg width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path
      d="M21 12a9 9 0 1 1-2.64-6.36"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
    />
    <path
      d="M21 3v6h-6"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

export const LibraryView = ({ useAlternativeGameCards, alternativeCardsOpacity, alternativeCardsFade }: LibraryViewProps) => {
  const { games, isLoading, status, setStatus, loadInstalledGames } = useLibraryGames();
  const [actionGame, setActionGame] = useState<InstalledGame | null>(null);
  const [infoGame, setInfoGame] = useState<InstalledGame | null>(null);
  const [versionGame, setVersionGame] = useState<InstalledGame | null>(null);
  const [manifestRows, setManifestRows] = useState<LuaManifestRow[]>([]);
  const [installFilter, setInstallFilter] = useState<LibraryInstallFilter>('all');
  const [filterReady, setFilterReady] = useState(false);

  const showStatus = (text: string, type: StatusType) => {
    setStatus({ text, type });
  };

  useEffect(() => {
    let cancelled = false;
    const loadFilter = async () => {
      try {
        const settings = await getSettings();
        if (!cancelled) {
          setInstallFilter(normalizeLibraryInstallFilter(settings.library_install_filter));
        }
      } catch {
        // Keep default "all" if settings cannot be read.
      } finally {
        if (!cancelled) setFilterReady(true);
      }
    };
    loadFilter();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (games.length > 0) {
      preloadGameCovers(games.map((game) => ({ appId: game.appId, imageUrl: game.imageUrl })), 60);
    }
  }, [games]);

  const filteredGames = useMemo(() => {
    if (installFilter === 'installed') {
      return games.filter((game) => game.installed);
    }
    if (installFilter === 'not_installed') {
      return games.filter((game) => !game.installed);
    }
    return games;
  }, [games, installFilter]);

  const persistInstallFilter = async (next: LibraryInstallFilter) => {
    try {
      const settings = await getSettings();
      await invoke('save_settings', {
        settings: {
          ...settings,
          library_install_filter: next,
        },
      });
    } catch (err: any) {
      setStatus({
        text: `Unable to save library filter: ${err}`,
        type: 'error',
      });
    }
  };

  const cycleInstallFilter = () => {
    const currentIndex = FILTER_CYCLE.indexOf(installFilter);
    const next = FILTER_CYCLE[(currentIndex + 1) % FILTER_CYCLE.length];
    setInstallFilter(next);
    void persistInstallFilter(next);
  };

  const handleOpenVersionEditor = async (game: InstalledGame) => {
    setStatus({ text: '', type: 'info' });

    try {
      const steamPath = await requireSteamPath();
      const rows: LuaManifestRow[] = await invoke('get_installed_lua_manifest_rows', {
        appId: Number(game.appId),
        steamPath,
      });

      setManifestRows((rows || []).map(row => ({ ...row, manifestInput: '' })));
      setVersionGame(game);
    } catch (err: any) {
      setStatus({
        text: `Unable to open version editor for ${game.name}: ${err}`,
        type: 'error',
      });
    }
  };

  const filterTitle =
    installFilter === 'installed'
      ? 'Showing installed Lua games only — click to show non-installed'
      : installFilter === 'not_installed'
        ? 'Showing non-installed Lua games only — click to show all'
        : 'Showing all Lua games — click to show installed only';

  const countLabel = (() => {
    if (isLoading || !filterReady) return 'Scanning Lua library...';
    const total = games.length;
    const shown = filteredGames.length;
    if (installFilter === 'all') {
      return `${total} Lua game${total === 1 ? '' : 's'} found`;
    }
    if (installFilter === 'installed') {
      return `${shown} installed Lua game${shown === 1 ? '' : 's'} (${total} total)`;
    }
    return `${shown} non-installed Lua game${shown === 1 ? '' : 's'} (${total} total)`;
  })();

  const emptyMessage = (() => {
    if (games.length === 0) {
      return 'No installed Steam games were found. Check your Steam path in Settings and press Refresh.';
    }
    if (installFilter === 'installed') {
      return 'No installed Lua games match this filter. Click the filter button or Refresh.';
    }
    if (installFilter === 'not_installed') {
      return 'No non-installed Lua games match this filter. Click the filter button or Refresh.';
    }
    return 'No installed Steam games were found. Check your Steam path in Settings and press Refresh.';
  })();

  return (
    <div className="store-view">
      <div className="store-header">
        <h1 className="store-title">Library</h1>
        <p className="store-subtitle">Manage every game Lua installed in Steam's stplug-in folder, with install status detected from Steam appmanifest ACF files.</p>
      </div>

      <div className="store-separator"></div>

      <StatusAlert status={status} />

      <div className="library-toolbar">
        <span className="library-count">{countLabel}</span>
        <div className="library-toolbar-actions">
          <button
            type="button"
            className={`library-icon-btn${installFilter !== 'all' ? ' active' : ''}${installFilter === 'not_installed' ? ' filter-missing' : ''}${installFilter === 'installed' ? ' filter-installed' : ''}`}
            onClick={cycleInstallFilter}
            disabled={isLoading || !filterReady}
            title={filterTitle}
            aria-label={filterTitle}
          >
            {installFilter === 'not_installed' ? <CloseIcon /> : <PlayIcon />}
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
        </div>
      </div>

      <div className="store-separator"></div>

      <div
        className={useAlternativeGameCards ? 'store-grid alt-card-grid' : 'store-grid'}
        style={useAlternativeGameCards ? {
          '--alt-card-opacity': Math.max(0, Math.min(100, alternativeCardsOpacity)),
          '--alt-card-fade': Math.max(0, Math.min(100, alternativeCardsFade)),
        } as React.CSSProperties : undefined}
      >
        {isLoading ? (
          <div className="store-no-results">Scanning Steam appmanifest files...</div>
        ) : filteredGames.length > 0 ? (
          filteredGames.map(game => (
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
          <div className="store-no-results">
            {emptyMessage}
          </div>
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
