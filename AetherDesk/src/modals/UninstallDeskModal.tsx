import { useState } from 'react';
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
 * AetherData. YES advances to the Steam-clean step (if residuals exist) or
 * starts the real folder removal.
 */
export const UninstallDeskConfirmModal = ({
  isProcessing,
  onConfirm,
  onCancel,
}: UninstallDeskConfirmModalProps) => {
  const [deleteUserData, setDeleteUserData] = useState(false);

  return (
    <ModalShell
      title="Uninstall AetherDesk"
      onClose={onCancel}
      closeDisabled={isProcessing}
      containerClassName="uninstall-modal"
      bodyClassName="uninstall-modal-body"
    >
      <p className="uninstall-modal-lead">
        Are you sure you want to uninstall AetherDesk?
      </p>
      <p className="uninstall-modal-copy">
        The portable application folder will be removed from this PC.
        {deleteUserData
          ? ' User data (settings, themes, wallpapers, backups) will be deleted too.'
          : ' User data will be kept next to the folder as AetherData.'}
      </p>

      <div className="uninstall-option-card">
        <label
          className="uninstall-userdata-row"
          title="If checked, AetherData is deleted with the app. If unchecked, it is moved next to the app folder before removal."
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

export interface UninstallSteamCleanModalProps {
  residualCount: number;
  isProcessing: boolean;
  /** YES — run Reset Path, then remove AetherDesk. */
  onConfirmClean: () => void;
  /** NO — skip Steam clean, still remove AetherDesk. */
  onSkipClean: () => void;
  /** X / Escape / overlay — abort the whole uninstall. */
  onCancel: () => void;
}

/**
 * Second step: only shown after YES on the confirm modal when Reset Path
 * targets still exist under Steam.
 */
export const UninstallSteamCleanModal = ({
  residualCount,
  isProcessing,
  onConfirmClean,
  onSkipClean,
  onCancel,
}: UninstallSteamCleanModalProps) => {
  const itemLabel =
    residualCount === 1 ? '1 residual item' : `${residualCount} residual items`;

  return (
    <ModalShell
      title="Clean Steam first?"
      onClose={onCancel}
      closeDisabled={isProcessing}
      containerClassName="uninstall-modal"
      bodyClassName="uninstall-modal-body"
    >
      <p className="uninstall-modal-lead">
        Aether still left {itemLabel} under your Steam path.
      </p>
      <p className="uninstall-modal-copy">
        These are the same targets Reset Path removes (DLLs,{' '}
        <code>aethercore</code> / <code>desk_path.cfg</code>, stplug-in,
        depotcache, and related files). Clean them before uninstalling?
      </p>

      <div className="uninstall-modal-actions">
        <button
          type="button"
          className="uninstall-btn uninstall-btn-primary"
          onClick={onConfirmClean}
          disabled={isProcessing}
        >
          {isProcessing ? 'Working…' : 'Yes, clean Steam'}
        </button>
        <button
          type="button"
          className="uninstall-btn uninstall-btn-secondary"
          onClick={onSkipClean}
          disabled={isProcessing}
        >
          No, just uninstall
        </button>
      </div>
    </ModalShell>
  );
};
