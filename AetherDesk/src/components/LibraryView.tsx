import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { GameCover } from './GameCover';
import { SpecificVersionModal, LuaManifestRow } from './SpecificVersionModal';

interface InstalledGame {
  id: number;
  name: string;
  appId: string;
  installDir: string;
  libraryPath: string;
  gamePath: string;
  installed: boolean;
  imageUrl?: string;
}

export const LibraryView = () => {
  const [games, setGames] = useState<InstalledGame[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [status, setStatus] = useState({ text: '', type: 'info' });
  const [versionGame, setVersionGame] = useState<InstalledGame | null>(null);
  const [manifestRows, setManifestRows] = useState<LuaManifestRow[]>([]);

  const loadInstalledGames = async () => {
    setIsLoading(true);
    setStatus({ text: '', type: 'info' });

    try {
      const result: InstalledGame[] = await invoke('get_installed_library_games');
      setGames(result || []);
    } catch (err: any) {
      setStatus({ text: `Failed to scan Steam library: ${err}`, type: 'error' });
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    loadInstalledGames();
  }, []);

  const handleModify = async (game: InstalledGame) => {
    setStatus({ text: '', type: 'info' });

    try {
      const settings: any = await invoke('get_settings');
      const steamPath = settings.steam_path;

      if (!steamPath || steamPath.trim() === '') {
        setStatus({ text: 'Please configure your Steam path in Settings first.', type: 'error' });
        return;
      }

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
        <p className="store-subtitle">Manage installed Steam games detected from appmanifest ACF files across your Steam libraries.</p>
      </div>

      <div className="store-separator"></div>

      {status.text && (
        <div className={`settings-alert ${status.type}`}>
          {status.text}
        </div>
      )}

      <div className="library-toolbar">
        <span className="library-count">
          {isLoading ? 'Scanning installed games...' : `${games.length} installed game${games.length === 1 ? '' : 's'} found`}
        </span>
        <button className="pagination-btn" onClick={loadInstalledGames} disabled={isLoading}>
          Refresh
        </button>
      </div>

      <div className="store-grid">
        {isLoading ? (
          <div className="store-no-results">Scanning Steam appmanifest files...</div>
        ) : games.length > 0 ? (
          games.map(game => (
            <div key={game.id} className="store-game-card">
              {game.installed && (
                <span className="badge-installed">Installed</span>
              )}

              <GameCover appId={game.appId} name={game.name} canonicalUrl={game.imageUrl} />

              <div className="game-info-wrapper">
                <div className="game-details">
                  <h3 className="game-name" title={game.name}>{game.name}</h3>
                  <span className="game-appid">App ID: {game.appId}</span>
                </div>
                <button
                  onClick={() => handleModify(game)}
                  className="game-download-btn"
                >
                  Modify
                </button>
              </div>
            </div>
          ))
        ) : (
          <div className="store-no-results">
            No installed Steam games were found. Check your Steam path in Settings and press Refresh.
          </div>
        )}
      </div>

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
