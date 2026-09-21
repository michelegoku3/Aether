import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { GameHeroImage } from '../ui/GameHeroImage';
import { WrenchIcon } from '../ui/icons';
import { useLibraryGames, type LuaManifestIssue } from '../hooks/useLibraryGames';
import { useModalDismiss } from '../hooks/useModalDismiss';
import { OnlinePanel, type OnlineStatus } from './OnlinePanel';
import { OnlineChoiceModal, type AppPresenceMode } from './OnlineChoiceModal';
import { resolveEffectivePresenceMode } from './onlineChoiceState';
import type { PresenceToggleArgs } from '../types/online';

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
  /** Malformed pins of this game's Lua, as reported by the Library scan. */
  luaIssues?: LuaManifestIssue[];
}

/** Report returned by the backend `sync_hubcap_game_manifest` command. */
interface GameManifestSyncReport {
  appId: number;
  pins: number;
  missing: number;
  generated: number;
  installed: number;
  skipped: boolean;
  /** Set when the Lua has a malformed pin: the repair cannot run until it is fixed. */
  invalidPinLine?: number | null;
}

/**
 * Human-readable one-liner for the per-game repair report, with its severity.
 *
 * A Lua that cannot load is a failure of the repair, not a success: it is
 * reported as an error so the Library alert shows it in red, and it names the
 * action that fixes it (Modify) instead of leaving the user at a dead end.
 */
const summarizeManifestRepair = (
  report: GameManifestSyncReport,
): { text: string; type: 'info' | 'success' | 'error' } => {
  if (report.invalidPinLine) {
    return {
      text: `This game's Lua has an invalid pin at line ${report.invalidPinLine}, so AetherDLL ignores the whole file. Open Modify and repair that line.`,
      type: 'error',
    };
  }
  if (report.skipped || report.pins === 0) {
    return { text: 'No manifest pins found for this game.', type: 'info' };
  }
  if (report.generated === 0) {
    return {
      text: `All ${report.pins} pinned manifest(s) are available locally; nothing to repair.`,
      type: 'success',
    };
  }
  return {
    text: `Repaired ${report.generated} of ${report.pins} pinned manifest(s) via Hubcap and installed ${report.installed} into depotcache.`,
    type: 'success',
  };
};

/**
 * Failure of the per-game update toggle, phrased for the person who has to
 * solve it.
 *
 * The backend refuses to enable updates while the Lua holds a malformed pin
 * (enabling them would comment the pins back in and hand Steam a file
 * AetherDLL cannot load), so the message must point at the repair instead of
 * leaving a raw command error with no next step.
 */
const describeUpdateToggleFailure = (error: string, enabling: boolean): string => {
  const action = enabling ? 'enable' : 'disable';
  if (/Invalid setManifestid call at Lua line (\d+)/.test(error)) {
    return `Cannot ${action} updates for this game: ${error} Open Modify and repair that line first.`;
  }
  return `Failed to update version pin state: ${error}`;
};

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
  const { games, queryGameState, invalidateGameState } = useLibraryGames();
  /**
   * Pins this game's Lua still needs repaired: malformed `setManifestid` calls
   * that the script engine executes, which is what makes AetherDLL reject the
   * whole file. Read from the current Library scan when the game is in it (the
   * object handed to the popup is a snapshot taken when the card was clicked),
   * falling back to that snapshot. Commented-out bad calls do not count: they
   * only come back when updates are enabled, and the button is disabled then.
   */
  const hasPinsToFix = (
    games.find((entry) => entry.appId === game.appId)?.luaIssues ?? game.luaIssues ?? []
  ).some((issue) => issue.active !== false);
  const [updatesEnabled, setUpdatesEnabled] = useState(false);
  const [isBusy, setIsBusy] = useState(false);
  const [showOnlineChoice, setShowOnlineChoice] = useState(false);
  const [showOnlinePanel, setShowOnlinePanel] = useState(false);
  const [onlineBusy, setOnlineBusy] = useState(false);
  const [aetherOnline, setAetherOnline] = useState(false);
  const [uco2Online, setUco2Online] = useState(false);
  const [ofmePresent, setOfmePresent] = useState(false);
  const [uco2FilesPresent, setUco2FilesPresent] = useState(false);
  const [showOnline, setShowOnline] = useState(false);
  const [aetherExcluded, setAetherExcluded] = useState(false);
  const [presenceDefaultShowOnline, setPresenceDefaultShowOnline] = useState(true);
  const disabled = isProcessing || isBusy || onlineBusy;
  // Un popup figlio (scelta online / pannello UCO2) aperto disattiva il
  // dismiss di QUESTO popup: ESC e click fuori chiudono solo il figlio in
  // cima alla catena Modify → Online → UCO2, che scende di un livello alla
  // volta senza mai chiudere tutto.
  const childPopupOpen = showOnlineChoice || showOnlinePanel;

  const refreshUpdateState = async () => {
    try {
      // Served through the shared provider cache: opening one game used to ask
      // the backend for this state several times over (StrictMode double mount,
      // the actions popup and the version editor each asking on their own).
      const state = await queryGameState<boolean>(Number(game.appId), 'get_lua_game_update_state');
      setUpdatesEnabled(Boolean(state));
    } catch {
      setUpdatesEnabled(false);
    }
  };

  useEffect(() => {
    refreshUpdateState();
  }, [game.appId]);

  useModalDismiss(onClose, disabled || childPopupOpen);

  const handleToggleUpdates = async () => {
    setIsBusy(true);
    const nextEnabled = !updatesEnabled;
    try {
      onStatus(nextEnabled ? 'Enabling updates for this game...' : 'Disabling updates for this game...', 'info');
      const result: string = await invoke('set_lua_game_updates_enabled', {
        appId: Number(game.appId),
        enabled: nextEnabled,
      });
      setUpdatesEnabled(nextEnabled);
      // This game's Lua just changed: drop its cached answers immediately
      // (the backend invalidation and the following scan would clear them a
      // moment later, but the toast must never describe stale rows).
      invalidateGameState(Number(game.appId));
      // The command emits a backend invalidation too; request the shared scan
      // directly for immediate feedback if the WebView event transport lags.
      onRefresh();
      onStatus(result, 'success');
    } catch (err: any) {
      onStatus(describeUpdateToggleFailure(String(err), nextEnabled), 'error');
    } finally {
      setIsBusy(false);
    }
  };

  const handleRepairManifests = async () => {
    setIsBusy(true);
    try {
      onStatus('Repairing manifests for this game...', 'info');
      // Local-first on the backend: backup/config hits are restored into
      // depotcache first; only genuinely absent manifests use the Hubcap key.
      const report = await invoke<GameManifestSyncReport>('sync_hubcap_game_manifest', {
        appId: Number(game.appId),
      });
      invalidateGameState(Number(game.appId));
      const summary = summarizeManifestRepair(report);
      onStatus(summary.text, summary.type);
    } catch (err: any) {
      onStatus(`Failed to repair manifests: ${err}`, 'error');
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
      const result: string = await invoke('remove_lua_game_from_library', {
        appId: Number(game.appId),
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
      const [aetherOnRaw, uco2Status, showOnRaw, excludedOnRaw, defaultShowOnline, foreign] = await Promise.all([
        invoke<boolean>('get_aetheronline', { appId: Number(game.appId) }),
        invoke<OnlineStatus>('get_online_status', { appId: Number(game.appId) }),
        invoke<boolean>('get_aether_showonline', { appId: Number(game.appId) }),
        invoke<boolean>('get_aether_excluded', { appId: Number(game.appId) }),
        invoke<boolean>('get_presence_default_mode'),
        invoke<{ ofme: boolean; uco2: boolean }>('inspect_foreign_online', { appId: Number(game.appId) }),
      ]);
      const aetherOn = Boolean(aetherOnRaw);
      const ofme = Boolean(foreign?.ofme);
      const uco2Files = Boolean(foreign?.uco2);
      const uco2On = uco2Status?.state === 'enabled' || uco2Files;
      const defaultOn = defaultShowOnline !== false;
      let showOn = Boolean(showOnRaw);
      let excludedOn = Boolean(excludedOnRaw);
      const spoof = ofme || uco2On;

      if (spoof && !aetherOn && !excludedOn && (showOn || defaultOn)) {
        try {
          await invoke<string>('set_aether_excluded', { appId: Number(game.appId), enabled: true } satisfies PresenceToggleArgs);
          excludedOn = true;
          showOn = false;
        } catch {
          // Display still falls back to None via `spoof`.
        }
      }

      setAetherOnline(aetherOn);
      setUco2Online(uco2On);
      setOfmePresent(ofme);
      setUco2FilesPresent(uco2Files);
      setShowOnline(showOn);
      setAetherExcluded(excludedOn);
      setPresenceDefaultShowOnline(defaultOn);
    } catch {
      // Keep the previous state on failure.
    }
  };

  const handleOpenOnline = async () => {
    await refreshOnlineStates();
    setShowOnlineChoice(true);
  };

  const currentPresenceMode: AppPresenceMode = resolveEffectivePresenceMode(
    aetherOnline,
    showOnline,
    aetherExcluded,
    presenceDefaultShowOnline,
    ofmePresent || uco2Online || uco2FilesPresent,
  );

  const handleSelectPresenceMode = async (next: AppPresenceMode) => {
    if (next === currentPresenceMode) return;
    setOnlineBusy(true);
    try {
      const command =
        next === 'aetheronline'
          ? 'set_aetheronline'
          : next === 'showonline'
            ? 'set_aether_showonline'
            : 'set_aether_excluded';
      // Un solo contratto di argomenti per i tre comandi: la shape resta
      // tipizzata anche se il nome del comando è scelto a runtime, così un
      // rename lato Rust rompe la compilazione invece del comportamento.
      const args: PresenceToggleArgs = { appId: Number(game.appId), enabled: true };
      const result: string = await invoke<string>(command, args);
      onStatus(result, 'success');
      await refreshOnlineStates();
    } catch (err: any) {
      onStatus(`Failed to set online mode: ${err}`, 'error');
    } finally {
      setOnlineBusy(false);
    }
  };

  const handleOpenUco2Panel = async () => {
    // UCO2 richiede None: Show Online sul wire 480 rompe gli inviti.
    if (currentPresenceMode !== 'none') {
      setOnlineBusy(true);
      try {
        await invoke<string>('set_aether_excluded', { appId: Number(game.appId), enabled: true } satisfies PresenceToggleArgs);
        await refreshOnlineStates();
      } catch (err: unknown) {
        onStatus(`Failed to switch to None for UCO2: ${err}`, 'error');
        setOnlineBusy(false);
        return;
      }
      setOnlineBusy(false);
    }
    setShowOnlineChoice(false);
    setShowOnlinePanel(true);
  };

  // Chiudere il pannello UCO2 NON chiude la catena: si torna al popup di
  // scelta online (con stati ricaricati, così il badge ACTIVE di UCO2 è
  // aggiornato), non al popup Modify e tantomeno alla libreria.
  const handleCloseUco2Panel = async () => {
    setShowOnlinePanel(false);
    await refreshOnlineStates();
    setShowOnlineChoice(true);
  };

  return (
    // Il click fuori è ignorato mentre un popup figlio è aperto: il click
    // appartiene a quel popup, che si chiude da solo e riporta qui.
    <div
      className="modal-overlay"
      onClick={disabled || childPopupOpen ? undefined : onClose}
    >
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
          <div className="game-action-row">
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
                  className={`game-action-btn${hasPinsToFix ? ' fix-pins-outline' : ''}`}
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
            <span
              className="game-action-repair-wrap"
              title="Restore from backup or regenerate via Hubcap every manifest referenced by this game's Lua"
            >
              <button
                className="game-action-btn game-action-repair-btn"
                onClick={handleRepairManifests}
                disabled={disabled}
                aria-label="Repair manifests"
                title="Repair manifests"
              >
                <WrenchIcon size={18} />
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
          ofmePresent={ofmePresent}
          uco2FilesPresent={uco2FilesPresent}
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
