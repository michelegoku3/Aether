import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ModalShell } from '../ui/ModalShell';
import { StatusType } from '../types/ui';

/** Minimal game shape for the bulk operation (satisfied by InstalledGame). */
export interface BulkUpdateGame {
  appId: string;
  name: string;
}

export interface GameUpdatesModalProps {
  /** Full library scan; every Lua entry is targeted, installed or not. */
  games: BulkUpdateGame[];
  /** Validated Steam root (caller resolves it via requireSteamPath). */
  steamPath: string;
  /** Toast feedback in the hosting view (same StatusAlert as other actions). */
  onStatus: (text: string, type: StatusType) => void;
  /** Single library rescan after the bulk run (immediate feedback). */
  onRefresh: () => void;
  /** X / Escape / overlay — dismiss (disabled while the bulk run executes). */
  onClose: () => void;
}

interface BulkProgress {
  action: 'block' | 'unblock';
  done: number;
  total: number;
}

/**
 * Block / unblock updates for every installed game in the library.
 *
 * Uses the exact per-game mechanism of the Modify popup
 * (`set_lua_game_updates_enabled`, manifest pins), applied sequentially to
 * each game: different games touch different manifest files, and
 * the library provider coalesces the resulting invalidation events into a
 * single follow-up scan, so no backend bulk command is needed.
 */
export const GameUpdatesModal = ({ games, steamPath, onStatus, onRefresh, onClose }: GameUpdatesModalProps) => {
  const [busy, setBusy] = useState<BulkProgress | null>(null);
  const [result, setResult] = useState<string | null>(null);

  const runBulk = async (enabled: boolean) => {
    if (busy || games.length === 0) return;
    const action = enabled ? 'unblock' : 'block';
    setBusy({ action, done: 0, total: games.length });
    setResult(null);
    onStatus(
      enabled
        ? `Enabling updates for ${games.length} games...`
        : `Disabling updates for ${games.length} games...`,
      'info'
    );
    const failed: string[] = [];
    let ok = 0;
    for (let i = 0; i < games.length; i++) {
      const game = games[i];
      try {
        await invoke('set_lua_game_updates_enabled', {
          appId: Number(game.appId),
          steamPath,
          enabled,
        });
        ok++;
      } catch {
        failed.push(game.name);
      }
      setBusy({ action, done: i + 1, total: games.length });
    }
    setBusy(null);
    // Same immediate-feedback refresh the Modify popup requests; the backend
    // invalidation events are coalesced by the provider, not re-scanned N times.
    onRefresh();
    const verb = enabled ? 'Unblocked updates' : 'Blocked updates';
    const summary =
      failed.length === 0
        ? `${verb} for ${ok} of ${games.length} games.`
        : `${verb} for ${ok} of ${games.length} games. Failed: ${failed.join(', ')}.`;
    setResult(summary);
    onStatus(summary, failed.length === 0 ? 'success' : 'error');
  };

  return (
    <ModalShell
      title="Game updates"
      onClose={() => { if (!busy) onClose(); }}
      closeDisabled={busy !== null}
      containerClassName="uninstall-modal"
      bodyClassName="uninstall-modal-body"
    >
      <p className="uninstall-modal-copy">
        Block or unblock updates for every Lua game in your library, using
        the same manifest pins as the per-game Enable / Disable Update button
        in Modify. Blocking restores the pins so Steam keeps each game on its
        current version; unblocking removes them so games update normally.
      </p>
      {games.length === 0 ? (
        <p className="uninstall-modal-copy">
          <strong>No Lua games found in the library.</strong>
        </p>
      ) : (
        <p className="uninstall-modal-copy">
          {games.length} Lua game{games.length === 1 ? '' : 's'} will be affected.
        </p>
      )}
      {busy && (
        <p className="uninstall-modal-copy">
          {busy.action === 'block' ? 'Blocking' : 'Unblocking'} updates… {busy.done}/{busy.total}
        </p>
      )}
      {!busy && result && (
        <p className="uninstall-modal-copy">{result}</p>
      )}

      <div className="uninstall-modal-actions">
        <button
          type="button"
          className="uninstall-btn uninstall-btn-primary"
          onClick={() => void runBulk(false)}
          disabled={busy !== null || games.length === 0}
        >
          {busy?.action === 'block' ? 'Blocking…' : 'Block all updates'}
        </button>
        <button
          type="button"
          className="uninstall-btn uninstall-btn-secondary"
          onClick={() => void runBulk(true)}
          disabled={busy !== null || games.length === 0}
        >
          {busy?.action === 'unblock' ? 'Unblocking…' : 'Unblock all updates'}
        </button>
      </div>
    </ModalShell>
  );
};
