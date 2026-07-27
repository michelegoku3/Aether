import { invoke } from '@tauri-apps/api/core';
import { GameHeroImage } from './GameHeroImage';

export interface LibraryActionGame {
  id: number;
  name: string;
  appId: string;
  installDir: string;
  libraryPath: string;
  gamePath: string;
  installed: boolean;
  imageUrl?: string;
}

interface LibraryGameActionsModalProps {
  game: LibraryActionGame;
  isProcessing: boolean;
  onClose: () => void;
  onStatus: (message: string, type: 'info' | 'success' | 'error') => void;
  onRefresh: () => void;
  onOpenVersionEditor: (game: LibraryActionGame) => void;
}

export const LibraryGameActionsModal = ({
  game,
  isProcessing,
  onClose,
  onStatus,
  onRefresh,
  onOpenVersionEditor,
}: LibraryGameActionsModalProps) => {
  const runWithSteamPath = async (
    action: (steamPath: string) => Promise<string>,
    pendingMessage: string,
  ) => {
    onStatus(pendingMessage, 'info');
    const settings: any = await invoke('get_settings');
    const steamPath = settings.steam_path;
    if (!steamPath || steamPath.trim() === '') {
      throw new Error('Please configure your Steam path in Settings first.');
    }
    return action(steamPath);
  };

  const handleEnableUpdates = async () => {
    try {
      const result = await runWithSteamPath(
        (steamPath) => invoke('enable_lua_game_updates', { appId: Number(game.appId), steamPath }),
        'Enabling Steam updates for this game...',
      );
      onStatus(result, 'success');
    } catch (err: any) {
      onStatus(`Failed to enable updates: ${err}`, 'error');
    }
  };

  const handleCleanCrack = async () => {
    try {
      onStatus('Cleaning generated crack/helper files...', 'info');
      const result: string = await invoke('clean_game_crack_files', { gamePath: game.gamePath });
      onStatus(result, 'success');
    } catch (err: any) {
      onStatus(`Failed to clean crack files: ${err}`, 'error');
    }
  };

  const handleRemove = async () => {
    try {
      const result = await runWithSteamPath(
        (steamPath) => invoke('remove_lua_game_from_library', { appId: Number(game.appId), steamPath }),
        'Removing Lua from Aether library...',
      );
      onStatus(result, 'success');
      onClose();
      onRefresh();
    } catch (err: any) {
      onStatus(`Failed to remove game from library: ${err}`, 'error');
    }
  };

  return (
    <div className="modal-overlay">
      <div className="modal-container game-action-modal">
        <GameHeroImage appId={game.appId} name={game.name} canonicalUrl={game.imageUrl} />

        <div className="game-action-body">
          <div className="game-action-grid">
            <button className="game-action-btn" onClick={handleEnableUpdates} disabled={isProcessing}>
              <span>Attiva</span>
              <span>aggiornamenti</span>
            </button>
            <button className="game-action-btn" onClick={() => onOpenVersionEditor(game)} disabled={isProcessing}>
              <span>Modifica</span>
              <span>versione</span>
            </button>
            <button className="game-action-btn" onClick={handleCleanCrack} disabled={isProcessing}>
              <span>Pulisci</span>
              <span>crack</span>
            </button>
            <button className="game-action-btn danger" onClick={handleRemove} disabled={isProcessing}>
              <span>Rimuovi</span>
            </button>
          </div>

          <button className="game-action-close-btn" onClick={onClose} disabled={isProcessing}>
            Chiudi
          </button>
        </div>
      </div>
    </div>
  );
};
