import { ModalShell } from '../ui/ModalShell';

export interface UnsavedChangesModalProps {
  /** Save in flight: dismiss gestures and buttons are disabled. */
  busy: boolean;
  /** User pressed "Save" — caller saves, then completes the pending action. */
  onSave: () => void;
  /** User pressed "Don't Save" — caller discards, then completes the action. */
  onDiscard: () => void;
  /** X / Escape / overlay — abort without changing anything (no Cancel
   *  button by design: the header X already covers it). */
  onCancel: () => void;
}

/**
 * Unsaved-changes guard for Settings, same style as the other app modals.
 *
 * Shown when the user switches tabs or closes the window with unsaved form
 * edits. While `busy` (save in flight) every dismiss path is disabled so the
 * pending action cannot be lost mid-save.
 */
export const UnsavedChangesModal = ({ busy, onSave, onDiscard, onCancel }: UnsavedChangesModalProps) => (
  <ModalShell
    title="Unsaved changes"
    onClose={() => { if (!busy) onCancel(); }}
    closeDisabled={busy}
    containerClassName="uninstall-modal"
    bodyClassName="uninstall-modal-body"
  >
    <p className="uninstall-modal-copy">
      You have unsaved changes in Settings. Save them before leaving?
    </p>

    <div className="uninstall-modal-actions">
      <button
        type="button"
        className="uninstall-btn uninstall-btn-primary"
        onClick={onSave}
        disabled={busy}
      >
        {busy ? 'Saving…' : 'Save'}
      </button>
      <button
        type="button"
        className="uninstall-btn uninstall-btn-danger"
        onClick={onDiscard}
        disabled={busy}
      >
        Don't Save
      </button>
    </div>
  </ModalShell>
);
