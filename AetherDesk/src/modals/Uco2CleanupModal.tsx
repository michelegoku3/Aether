import { ModalShell } from '../ui/ModalShell';

export interface Uco2CleanupModalProps {
  /** `confirm` asks before the destructive cleanup; `warning` reports it after. */
  mode: 'confirm' | 'warning';
  /** Backend-provided text (confirmation question or verify-integrity warning). */
  text: string;
  /** Cleanup in flight: dismiss gestures and buttons are disabled. */
  busy?: boolean;
  /** `confirm` mode: proceed with removing the known UCO2 files. */
  onConfirm?: () => void;
  /** Cancel (confirm) or acknowledge (warning). */
  onClose: () => void;
}

/**
 * Journal-less UCOnline2 cleanup dialogs.
 *
 * Disable normally rolls back from the recovery journal. When the whole
 * AetherData folder is gone the journal is lost, so AetherDesk can only remove
 * the *known* UCO2 files — some may be legitimate game files. The confirm step
 * makes that explicit; the warning step tells the user how to restore them by
 * verifying the game file integrity.
 */
export const Uco2CleanupModal = ({ mode, text, busy = false, onConfirm, onClose }: Uco2CleanupModalProps) => (
  <ModalShell
    title={mode === 'confirm' ? 'Remove UCOnline2 files?' : 'UCOnline2 files removed'}
    onClose={() => {
      if (!busy) onClose();
    }}
    closeDisabled={busy}
    containerClassName="uninstall-modal"
    bodyClassName="uninstall-modal-body"
  >
    <p className="uninstall-modal-copy">{text}</p>

    <div className="uninstall-modal-actions">
      {mode === 'confirm' ? (
        <>
          <button type="button" className="uninstall-btn uninstall-btn-danger" onClick={onConfirm} disabled={busy}>
            {busy ? 'Removing…' : 'Continue'}
          </button>
          <button type="button" className="uninstall-btn uninstall-btn-secondary" onClick={onClose} disabled={busy}>
            Cancel
          </button>
        </>
      ) : (
        <button type="button" className="uninstall-btn uninstall-btn-primary" onClick={onClose}>
          Close
        </button>
      )}
    </div>
  </ModalShell>
);
