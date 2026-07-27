import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { SpecificVersionModal, LuaManifestRow } from './SpecificVersionModal';
import { LibraryGameActionsModal } from './LibraryGameActionsModal';
import { GameCard } from './ui/GameCard';
import { StatusAlert } from './ui/StatusAlert';
import { useLibraryGames, InstalledGame } from '../hooks/useLibraryGames';
import { StatusType } from '../types/ui';
import { requireSteamPath } from '../hooks/useSettings';

export const LibraryView = () => {
  const { games, isLoading, status, setStatus, loadInstalledGames } = useLibraryGames();
  const [actionGame, setActionGame] = useState<InstalledGame | null>(null);
  const [versionGame, setVersionGame] = useState<InstalledGame | null>(null);
  const [manifestRows, setManifestRows] = useState<LuaManifestRow[]>([]);

  const showStatus = (text: string, type: StatusType) => {
    setStatus({ text, type });
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

  return (
    <div className="store-view">
      <div className="store-header">
        <h1 className="store-title">Library</h1>
        <p className="store-subtitle">Manage every game Lua installed in Steam's stplug-in folder, with install status detected from Steam appmanifest ACF files.</p>
      </div>

      <div className="store-separator"></div>

      <StatusAlert status={status} />

      <div className="library-toolbar">
        <span className="library-count">
          {isLoading ? 'Scanning Lua library...' : `${games.length} Lua game${games.length === 1 ? '' : 's'} found`}
        </span>
        <button className="pagination-btn" onClick={loadInstalledGames} disabled={isLoading}>
          Refresh
        </button>
      </div>

      <div className="store-separator"></div>

      <div className="store-grid">
        {isLoading ? (
          <div className="store-no-results">Scanning Steam appmanifest files...</div>
        ) : games.length > 0 ? (
          games.map(game => (
            <GameCard
              key={game.id}
              game={game}
              actionLabel="Modify"
              onAction={setActionGame}
            />
          ))
        ) : (
          <div className="store-no-results">
            No installed Steam games were found. Check your Steam path in Settings and press Refresh.
          </div>
        )}
      </div>

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
