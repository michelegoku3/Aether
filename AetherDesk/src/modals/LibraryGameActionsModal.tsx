import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { GameHeroImage } from '../ui/GameHeroImage';
import { requireSteamPath } from '../hooks/useSettings';
import { useModalDismiss } from '../hooks/useModalDismiss';
import { OnlinePanel } from './OnlinePanel';
import { OnlineChoiceModal } from './OnlineChoiceModal';

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
  const [showOnlineChoice, setShowOnlineChoice] = useState(false);
  const [showOnlinePanel, setShowOnlinePanel] = useState(false);
  const [onlineBusy, setOnlineBusy] = useState(false);
  const [aetherOnlinefix, setAetherOnlinefix] = useState(false);
  const [uco2Online, setUco2Online] = useState(false);
  const [showOnline, setShowOnline] = useState(false);
  const disabled = isProcessing || isBusy || onlineBusy;

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

  // ESC + click fuori chiudono il popup (rispettando le operazioni in corso).
  useModalDismiss(onClose, disabled);

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

  const refreshOnlineStates = async () => {
    try {
      // Parallel check: Aether via LaunchOptions (-onlinefix / -showonline),
      // UCO2 via the presence of union-crax.ini next to the game executable.
      const [aetherOn, uco2On, showOn] = await Promise.all([
        invoke<boolean>('get_aether_onlinefix', { appId: Number(game.appId) }),
        invoke<boolean>('is_uco2_active', { appId: Number(game.appId) }),
        invoke<boolean>('get_aether_showonline', { appId: Number(game.appId) }),
      ]);
      setAetherOnlinefix(Boolean(aetherOn));
      setUco2Online(Boolean(uco2On));
      setShowOnline(Boolean(showOn));
    } catch {
      // Keep the previous state on failure.
    }
  };

  const handleOpenOnline = async () => {
    await refreshOnlineStates();
    setShowOnlineChoice(true);
  };

  const handleToggleAether = async () => {
    setOnlineBusy(true);
    try {
      const nextEnabled = !aetherOnlinefix;
      const result: string = await invoke('set_aether_onlinefix', {
        appId: Number(game.appId),
        enabled: nextEnabled,
      });
      onStatus(result, 'success');
      setAetherOnlinefix(nextEnabled);
      if (nextEnabled) {
        setUco2Online(false);
        setShowOnline(false);
      }
    } catch (err: any) {
      onStatus(`Failed to toggle Aether onlinefix: ${err}`, 'error');
    } finally {
      setOnlineBusy(false);
    }
  };

  const handleToggleShowOnline = async () => {
    setOnlineBusy(true);
    try {
      const nextEnabled = !showOnline;
      // -showonline and -onlinefix are mutually exclusive; the backend strips
      // the other token automatically when one is enabled.
      const result: string = await invoke('set_aether_showonline', {
        appId: Number(game.appId),
        enabled: nextEnabled,
      });
      onStatus(result, 'success');
      setShowOnline(nextEnabled);
      if (nextEnabled) {
        setAetherOnlinefix(false);
      }
    } catch (err: any) {
      onStatus(`Failed to toggle Show Online: ${err}`, 'error');
    } finally {
      setOnlineBusy(false);
    }
  };

  const handleOpenUco2Panel = () => {
    setShowOnlineChoice(false);
    setShowOnlinePanel(true);
  };

  const handleCloseUco2Panel = async () => {
    setShowOnlinePanel(false);
    await refreshOnlineStates();
  };

  return (
    <div className="modal-overlay" onClick={disabled ? undefined : onClose}>
      <div className="modal-container game-action-modal" onClick={(e) => e.stopPropagation()}>
        <div className="game-action-hero-wrap">
          <GameHeroImage appId={game.appId} name={game.name} canonicalUrl={game.heroImageUrl || game.imageUrl} />
          <button
            type="button"
            className="game-action-hero-close"
            onClick={onClose}
            disabled={disabled}
            aria-label="Close"
            title="Close"
          >
            <svg
              viewBox="0 0 24 24"
              width="16"
              height="16"
              aria-hidden="true"
              focusable="false"
            >
              <path
                d="M5 5 L19 19 M19 5 L5 19"
                stroke="currentColor"
                strokeWidth="2.6"
                strokeLinecap="round"
                fill="none"
              />
            </svg>
          </button>
        </div>

        <div className="game-action-body">
          <div className="game-action-grid">
            {/* Each button is wrapped so the tooltip still appears when the
                button itself is disabled (Chromium/WebView2 suppress hover on
                disabled controls, so the title must live on the wrapper). */}
            <span className="game-action-btn-wrap">
              <button className="game-action-btn" onClick={handleToggleUpdates} disabled={disabled}>
                {updatesEnabled ? 'Disable Update' : 'Enable Update'}
              </button>
            </span>
            <span
              className="game-action-btn-wrap"
              title={updatesEnabled ? 'Disable updates for this game' : undefined}
            >
              <button
                className="game-action-btn"
                onClick={() => onOpenVersionEditor(game)}
                disabled={disabled || updatesEnabled}
              >
                Change Version
              </button>
            </span>
            <span
              className="game-action-btn-wrap"
              title={!game.installed ? 'Online requires the game to be installed in Steam first' : undefined}
            >
              <button
                className="game-action-btn"
                onClick={handleOpenOnline}
                disabled={disabled || !game.installed}
              >
                ONLINE
              </button>
            </span>
            <span
              className="game-action-btn-wrap"
              title={
                !game.installed
                  ? 'Show Online requires the game to be installed in Steam first'
                  : showOnline
                    ? 'Stop showing this game to your Steam friends (singleplayer presence)'
                    : 'Show your Steam friends what you are playing (singleplayer, no online features)'
              }
            >
              <button
                className="game-action-btn"
                onClick={handleToggleShowOnline}
                disabled={disabled || onlineBusy || !game.installed}
              >
                {showOnline ? 'Disable Show Online' : 'Show Online'}
              </button>
            </span>
            <span
              className="game-action-btn-wrap"
              title={game.installed ? 'Installed games cannot be removed from Aether Library' : 'Remove Lua from Aether Library'}
            >
              <button
                className="game-action-btn danger"
                onClick={handleRemove}
                disabled={disabled || game.installed}
              >
                Remove
              </button>
            </span>
          </div>
        </div>
      </div>

      {showOnlineChoice && (
        <OnlineChoiceModal
          game={game}
          aetherEnabled={aetherOnlinefix}
          uco2Enabled={uco2Online}
          busy={onlineBusy}
          onToggleAether={handleToggleAether}
          onOpenUco2Panel={handleOpenUco2Panel}
          onClose={() => setShowOnlineChoice(false)}
        />
      )}
      {showOnlinePanel && <OnlinePanel game={game} onClose={handleCloseUco2Panel} />}
    </div>
  );
};
