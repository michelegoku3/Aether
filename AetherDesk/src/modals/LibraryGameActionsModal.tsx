import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { GameHeroImage } from '../ui/GameHeroImage';
import { requireSteamPath } from '../hooks/useSettings';

export interface LibraryActionGame {
  id: number;
  name: string;
  appId: string;
  installDir: string;
  libraryPath: string;
  gamePath: string;
  installed: boolean;
  imageUrl?: string;
  heroImageUrl?: string;
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
  const [updatesEnabled, setUpdatesEnabled] = useState(false);
  const [isBusy, setIsBusy] = useState(false);
  const disabled = isProcessing || isBusy;

  const refreshUpdateState = async () => {
    try {
      const steamPath = await requireSteamPath();
      const state: boolean = await invoke('get_lua_game_update_state', {
        appId: Number(game.appId),
        steamPath,
      });
      setUpdatesEnabled(Boolean(state));
    } catch {
      setUpdatesEnabled(false);
    }
  };

  useEffect(() => {
    refreshUpdateState();
  }, [game.appId]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !disabled) {
        onClose();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [disabled, onClose]);

  const handleToggleUpdates = async () => {
    setIsBusy(true);
    try {
      const nextEnabled = !updatesEnabled;
      onStatus(nextEnabled ? 'Enabling updates for this game...' : 'Disabling updates for this game...', 'info');
      const steamPath = await requireSteamPath();
      const result: string = await invoke('set_lua_game_updates_enabled', {
        appId: Number(game.appId),
        steamPath,
        enabled: nextEnabled,
      });
      setUpdatesEnabled(nextEnabled);
      onStatus(result, 'success');
    } catch (err: any) {
      onStatus(`Failed to update version pin state: ${err}`, 'error');
    } finally {
      setIsBusy(false);
    }
  };

  const handleRemove = async () => {
    if (game.installed) {
      onStatus('Remove is available only for games that are not installed in Steam.', 'error');
      return;
    }

    setIsBusy(true);
    try {
      onStatus('Removing Lua from Aether library...', 'info');
      const steamPath = await requireSteamPath();
      const result: string = await invoke('remove_lua_game_from_library', {
        appId: Number(game.appId),
        steamPath,
      });
      onStatus(result, 'success');
      onClose();
      onRefresh();
    } catch (err: any) {
      onStatus(`Failed to remove game from library: ${err}`, 'error');
    } finally {
      setIsBusy(false);
    }
  };

  return (
    <div className="modal-overlay">
      <div className="modal-container game-action-modal">
        <GameHeroImage appId={game.appId} name={game.name} canonicalUrl={game.heroImageUrl || game.imageUrl} />

        <div className="game-action-body">
          <div className="game-action-grid">
            <button className="game-action-btn" onClick={handleToggleUpdates} disabled={disabled}>
              {updatesEnabled ? 'Disable Update' : 'Enable Update'}
            </button>
            <button className="game-action-btn" onClick={() => onOpenVersionEditor(game)} disabled={disabled}>
              Change Version
            </button>
            <button className="game-action-btn" disabled title="Enable Online is not available yet.">
              Enable Online
            </button>
            <button
              className="game-action-btn danger"
              onClick={handleRemove}
              disabled={disabled || game.installed}
              title={game.installed ? 'Installed games cannot be removed from Aether Library.' : 'Remove Lua from Aether Library'}
            >
              Remove
            </button>
          </div>

          <button className="game-action-close-btn" onClick={onClose} disabled={disabled}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
};
