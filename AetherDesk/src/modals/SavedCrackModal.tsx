import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ModalShell } from '../ui/ModalShell';
import type { CrackTargetGame } from './CrackModal';

export interface SavedCrackModalProps {
  game: CrackTargetGame;
  /**
   * YES — saved crack was re-applied successfully.
   * Parent should show the status and close the flow.
   */
  onReapplied: (message: string) => void;
  /**
   * NO — applied crack was removed from the game; open the normal drop modal.
   */
  onDeclined: (cleanupMessage?: string) => void;
  /** X / Escape / overlay — abort without changing the game. */
  onCancel: () => void;
}

/**
 * Shown when Apply Crack finds files under `AetherData/backup/<appId>/crack/`.
 * YES reuses that backup; NO strips the crack from the game and continues to
 * the drop-zone modal for a fresh apply.
 */
export const SavedCrackModal = ({
  game,
  onReapplied,
  onDeclined,
  onCancel,
}: SavedCrackModalProps) => {
  const [isBusy, setIsBusy] = useState(false);
  const [error, setError] = useState('');

  const handleYes = async () => {
    setIsBusy(true);
    setError('');
    try {
      const message: string = await invoke('reapply_saved_crack', {
        appId: Number(game.appId),
      });
      onReapplied(message);
    } catch (err: any) {
      setError(String(err));
      setIsBusy(false);
    }
  };

  const handleNo = async () => {
    setIsBusy(true);
    setError('');
    try {
      const message: string = await invoke('remove_applied_crack', {
        appId: Number(game.appId),
      });
      onDeclined(message);
    } catch (err: any) {
      // Still open the drop modal — cleanup failure should not trap the user.
      onDeclined(`Could not fully remove the previous crack: ${err}`);
    }
  };

  return (
    <ModalShell
      title="Saved crack found"
      onClose={onCancel}
      closeDisabled={isBusy}
      containerClassName="uninstall-modal"
      bodyClassName="uninstall-modal-body"
    >
      <p className="uninstall-modal-lead">
        A crack is already saved for{' '}
        <strong style={{ color: '#ffffff' }}>{game.name}</strong>.
      </p>
      <p className="uninstall-modal-copy">
        Do you want to apply the saved crack from your Aether backup?
        Choosing No removes the crack currently in the game folder and opens
        the usual file drop dialog.
      </p>

      {error && (
        <div className="settings-alert error settings-alert--compact">
          {error}
        </div>
      )}

      <div className="uninstall-modal-actions">
        <button
          type="button"
          className="uninstall-btn uninstall-btn-primary"
          onClick={handleYes}
          disabled={isBusy}
        >
          {isBusy ? 'Working…' : 'Yes, use saved'}
        </button>
        <button
          type="button"
          className="uninstall-btn uninstall-btn-secondary"
          onClick={handleNo}
          disabled={isBusy}
        >
          No, choose new
        </button>
      </div>
    </ModalShell>
  );
};
