import { ModalShell } from '../ui/ModalShell';

export interface CleanupModalProps {
  /** `confirm` asks before the destructive cleanup; `warning` reports it after. */
  mode: 'confirm' | 'warning';
  /** Dialog title (e.g. "Remove UCOnline2 files?"). */
  title: string;
  /** Body text: the confirmation question or the verify-integrity warning. */
  text: string;
  /** Cleanup in flight: dismiss gestures and buttons are disabled. */
  busy?: boolean;
  /** `confirm` mode: proceed with removing the detected files. */
  onConfirm?: () => void;
  /** Cancel (confirm) or acknowledge (warning). */
  onClose: () => void;
}

/**
 * Generic destructive-cleanup dialogs: confirm, then report the result.
 *
 * Shared by the UCOnline2 journal-less disable and the OFME "Clean" button so
 * both use the same wording, layout and two-step flow. The text is provided by
 * the caller (the backend owns the exact wording).
 */
export const CleanupModal = ({ mode, title, text, busy = false, onConfirm, onClose }: CleanupModalProps) => (
  <ModalShell
    title={title}
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
