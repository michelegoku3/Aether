import { ModalShell } from '../ui/ModalShell';

interface AutoUpdateReadyModalProps {
  tag: string;
  onClose: () => void;
  onRestart: () => void;
}

/** Shown when the background auto-updater has staged a newer AetherDesk
 * release and is waiting for the user to restart the app to apply it. */
export const AutoUpdateReadyModal = ({ tag, onClose, onRestart }: AutoUpdateReadyModalProps) => (
  <ModalShell
    title="AetherDesk update ready"
    onClose={onClose}
    containerClassName="uninstall-modal"
    bodyClassName="uninstall-modal-body"
  >
    <p className="uninstall-modal-copy">
      AetherDesk has downloaded and staged update <strong>{tag}</strong> in the background. Restart the app to apply it.
    </p>
    <div className="uninstall-modal-actions">
      <button type="button" className="uninstall-btn uninstall-btn-primary" onClick={onRestart}>
        Restart now
      </button>
      <button type="button" className="uninstall-btn" onClick={onClose}>
        Remind me later
      </button>
    </div>
  </ModalShell>
);
