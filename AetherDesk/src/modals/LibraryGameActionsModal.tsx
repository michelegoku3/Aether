import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { GameHeroImage } from '../ui/GameHeroImage';
import { requireSteamPath } from '../hooks/useSettings';
import { useModalDismiss } from '../hooks/useModalDismiss';
import { OnlinePanel, type OnlineStatus } from './OnlinePanel';
import { OnlineChoiceModal, type AppPresenceMode } from './OnlineChoiceModal';

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
  const [aetherExcluded, setAetherExcluded] = useState(false);
  const [presenceDefaultShowOnline, setPresenceDefaultShowOnline] = useState(true);
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
      // Parallel checks. UCO2 viene letto da get_online_status — il record di
      // stato scritto dal pannello (Engine::status: record presente + ini/dll
      // effettivamente sul disco). NON is_uco2_active: quel probe legge
      // union-crax.ini, un file di RUNTIME che UCO2 lascia sul disco anche
      // dopo il Disable → falso "sempre attivo" che bloccava Online Aether.
      // 'broken' (record senza file) conta come non attivo ai fini del gate.
      const [aetherOn, uco2Status, showOn, excludedOn, defaultShowOnline] = await Promise.all([
        invoke<boolean>('get_aether_onlinefix', { appId: Number(game.appId) }),
        invoke<OnlineStatus>('get_online_status', { appId: Number(game.appId) }),
        invoke<boolean>('get_aether_showonline', { appId: Number(game.appId) }),
        invoke<boolean>('get_aether_excluded', { appId: Number(game.appId) }),
        invoke<boolean>('get_presence_default_mode'),
      ]);
      setAetherOnlinefix(Boolean(aetherOn));
      setUco2Online(uco2Status?.state === 'enabled');
      setShowOnline(Boolean(showOn));
      setAetherExcluded(Boolean(excludedOn));
      setPresenceDefaultShowOnline(defaultShowOnline !== false);
    } catch {
      // Keep the previous state on failure.
    }
  };

  const handleOpenOnline = async () => {
    await refreshOnlineStates();
    setShowOnlineChoice(true);
  };

  // Modalità EFFETTIVA per il popup (docs/05 §12): un'app senza entry nelle
  // liste ricade in default_mode — mostrato come la scelta corrente, così
  // l'utente vede lo stato reale e non quello formale.
  const currentPresenceMode: AppPresenceMode = aetherOnlinefix
    ? 'onlinefix'
    : showOnline
      ? 'showonline'
      : aetherExcluded
        ? 'none'
        : presenceDefaultShowOnline
          ? 'showonline'
          : 'none';

  const handleSelectPresenceMode = async (next: AppPresenceMode) => {
    if (next === currentPresenceMode) return;
    setOnlineBusy(true);
    try {
      const command =
        next === 'onlinefix'
          ? 'set_aether_onlinefix'
          : next === 'showonline'
            ? 'set_aether_showonline'
            : 'set_aether_excluded';
      const result: string = await invoke(command, {
        appId: Number(game.appId),
        enabled: true,
      });
      onStatus(result, 'success');
      await refreshOnlineStates();
    } catch (err: any) {
      onStatus(`Failed to set online mode: ${err}`, 'error');
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
          mode={currentPresenceMode}
          uco2Enabled={uco2Online}
          busy={onlineBusy}
          onSelectMode={handleSelectPresenceMode}
          onOpenUco2Panel={handleOpenUco2Panel}
          onClose={() => setShowOnlineChoice(false)}
        />
      )}
      {showOnlinePanel && <OnlinePanel game={game} onClose={handleCloseUco2Panel} />}
    </div>
  );
};
