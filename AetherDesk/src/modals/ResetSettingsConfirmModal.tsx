import { ModalShell } from '../ui/ModalShell';

export interface ResetSettingsConfirmModalProps {
  /** Reset in flight: dismiss gestures and both buttons are disabled. */
  busy: boolean;
  /** User confirmed: replaces the saved settings with defaults. */
  onConfirm: () => void;
  /** X / Escape / overlay / NO — keep the current settings. */
  onCancel: () => void;
}

/**
 * Destructive gate for the Settings screen's Reset button. Same chrome and
 * action row as the other confirmation dialogs (Uninstall, Reset Steam path).
 */
export const ResetSettingsConfirmModal = ({ busy, onConfirm, onCancel }: ResetSettingsConfirmModalProps) => (
  <ModalShell
    title="Reset settings?"
    onClose={onCancel}
    closeDisabled={busy}
    containerClassName="uninstall-modal"
    bodyClassName="uninstall-modal-body"
  >
    <p className="uninstall-modal-lead">Are you sure you want to reset Settings?</p>
    <p className="uninstall-modal-copy">Every setting returns to its default value. This cannot be undone.</p>

    <div className="uninstall-modal-actions">
      <button type="button" className="uninstall-btn uninstall-btn-danger" onClick={onConfirm} disabled={busy}>
        {busy ? 'Working…' : 'Yes'}
      </button>
      <button type="button" className="uninstall-btn uninstall-btn-secondary" onClick={onCancel} disabled={busy}>
        No
      </button>
    </div>
  </ModalShell>
);
