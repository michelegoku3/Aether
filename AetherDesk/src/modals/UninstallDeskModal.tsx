import { useEffect, useState } from 'react';
import { ModalShell } from '../ui/ModalShell';

export interface UninstallDeskConfirmModalProps {
  isProcessing: boolean;
  /** User confirmed uninstall. `deleteUserData` mirrors the checkbox. */
  onConfirm: (deleteUserData: boolean) => void;
  /** X / Escape / overlay / NO — abort. */
  onCancel: () => void;
}

/**
 * First step of portable Uninstall: "Are you sure?" plus optional wipe of
 * AetherData. YES advances to the Reset-Steam-path confirm (if residuals
 * exist) or starts removal of verified application files.
 */
export const UninstallDeskConfirmModal = ({ isProcessing, onConfirm, onCancel }: UninstallDeskConfirmModalProps) => {
  const [deleteUserData, setDeleteUserData] = useState(false);

  return (
    <ModalShell
      title="Uninstall AetherDesk"
      onClose={onCancel}
      closeDisabled={isProcessing}
      containerClassName="uninstall-modal"
      bodyClassName="uninstall-modal-body"
    >
      <p className="uninstall-modal-lead">Are you sure you want to uninstall AetherDesk?</p>
      <p className="uninstall-modal-copy">
        Only verified AetherDesk application files will be removed. Unrelated or modified files will be kept.
        {deleteUserData
          ? ' User data (settings, themes, wallpapers, backups) will be deleted too.'
          : ' User data will stay in AetherData at its current location.'}
      </p>

      <div className="uninstall-option-card">
        <label
          className="uninstall-userdata-row"
          title="If checked, AetherData is deleted. If unchecked, it stays at its current location. Other files are never deleted."
        >
          <span className="uninstall-userdata-text">Delete user data?</span>
          <input
            type="checkbox"
            className="crack-checkbox-input"
            checked={deleteUserData}
            disabled={isProcessing}
            onChange={(e) => setDeleteUserData(e.target.checked)}
          />
          <span className="crack-checkbox-box" aria-hidden="true" />
        </label>
      </div>

      <div className="uninstall-modal-actions">
        <button
          type="button"
          className="uninstall-btn uninstall-btn-danger"
          onClick={() => onConfirm(deleteUserData)}
          disabled={isProcessing}
        >
          {isProcessing ? 'Working…' : 'Yes'}
        </button>
        <button
          type="button"
          className="uninstall-btn uninstall-btn-secondary"
          onClick={onCancel}
          disabled={isProcessing}
        >
          No
        </button>
      </div>
    </ModalShell>
  );
};

export interface ResetSteamConfirmModalProps {
  isProcessing: boolean;
  /** User waited out the countdown and confirmed. Runs Reset Path. */
  onConfirm: () => void;
  /** X / Escape / overlay — abort, nothing is deleted. Also the NO handler
   *  when no `onReject` is given (standalone Reset Path gate). */
  onCancel: () => void;
  /**
   * Uninstall flow only: NO means "skip the Steam clean, still uninstall".
   * When set, the NO button switches to "No, just uninstall" and stops being
   * a dismiss gesture; X / Escape / overlay keep aborting via `onCancel`.
   */
  onReject?: () => void;
}

/** Seconds the destructive YES stays disabled after the popup opens. */
const RESET_CONFIRM_DELAY_S = 10;

/**
 * Reset Path gate: resetting the Steam path cannot be undone — it deletes
 * every Aether-created file and folder under the Steam directory (DLLs, all
 * .lua in config/stplug-in, all .manifest in depotcache). The YES button
 * stays disabled for a 10-second countdown so the warning is actually read.
 * Dismiss gestures (X / Escape / overlay / NO) stay available the whole
 * time; only a running reset locks the dialog. In the uninstall flow the
 * dialog doubles as the Steam-clean gate: there NO (`onReject`) skips the
 * clean and keeps uninstalling.
 */
export const ResetSteamConfirmModal = ({
  isProcessing,
  onConfirm,
  onCancel,
  onReject,
}: ResetSteamConfirmModalProps) => {
  const [remaining, setRemaining] = useState(RESET_CONFIRM_DELAY_S);

  useEffect(() => {
    if (remaining <= 0) return;
    const timer = window.setTimeout(() => setRemaining((value) => Math.max(0, value - 1)), 1000);
    return () => window.clearTimeout(timer);
  }, [remaining]);

  return (
    <ModalShell
      title="Reset Steam path?"
      onClose={onCancel}
      closeDisabled={isProcessing}
      containerClassName="uninstall-modal"
      bodyClassName="uninstall-modal-body"
    >
      <p className="uninstall-modal-lead">Resetting the Steam path cannot be undone.</p>
      <p className="uninstall-modal-copy">
        Everything Aether created under your Steam folder will be deleted: the DLLs (<code>AetherCore.dll</code>,{' '}
        <code>AetherPayload.dll</code>, <code>xinput1_4.dll</code>), the <code>aethercore</code> and{' '}
        <code>config/stplug-in</code> folders (all your .lua files), and the <code>depotcache</code> contents (all your
        .manifest files).
      </p>

      <div className="uninstall-modal-actions">
        <button
          type="button"
          className="uninstall-btn uninstall-btn-danger"
          onClick={onConfirm}
          disabled={isProcessing || remaining > 0}
        >
          {isProcessing ? 'Working…' : remaining > 0 ? `Yes, clean Steam (${remaining})` : 'Yes, clean Steam'}
        </button>
        <button
          type="button"
          className="uninstall-btn uninstall-btn-secondary"
          onClick={onReject ?? onCancel}
          disabled={isProcessing}
        >
          {onReject ? 'No, just uninstall' : 'No'}
        </button>
      </div>
    </ModalShell>
  );
};
