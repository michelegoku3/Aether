import { ModalShell } from '../ui/ModalShell';

interface HubcapUpdateWarningModalProps {
  onClose: () => void;
}

/** Shown on every attempt to enable latest-download updates without a
 * positively validated Hubcap key. There is intentionally no acknowledgement
 * state: the requirement must be re-evaluated each time. */
export const HubcapUpdateWarningModal = ({ onClose }: HubcapUpdateWarningModalProps) => (
  <ModalShell
    title="Hubcap key required for updates"
    onClose={onClose}
    containerClassName="uninstall-modal"
    bodyClassName="uninstall-modal-body"
  >
    <p className="uninstall-modal-copy">
      Steam has patched the unauthenticated request code. Without authenticated
      manifest access, automatic updates and Workshop downloads are no longer
      reliable.
    </p>
    <p className="uninstall-modal-copy">
      Enable “Download games with updates on” only after entering and saving a
      valid active Hubcap key. The option has been left OFF.
    </p>
    <div className="uninstall-modal-actions">
      <button type="button" className="uninstall-btn uninstall-btn-primary" onClick={onClose}>
        I understand
      </button>
    </div>
  </ModalShell>
);
