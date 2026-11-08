import { ModalShell } from '../ui/ModalShell';

export interface HubcapKeyExpiredModalProps {
  /** Provider verdict: account inactive or credentials rejected. */
  reason?: string;
  /** Switch to Settings and close. */
  onGoToSettings: () => void;
  /** X / Escape / overlay / "Later". */
  onClose: () => void;
}

const detailFor = (reason?: string): string => {
  if (reason === 'account_inactive') {
    return 'The Hubcap account behind the saved key is no longer allowed to make requests.';
  }
  if (reason === 'unauthorized') {
    return 'Hubcap rejected the saved API key: it is invalid, revoked or expired.';
  }
  return 'The saved Hubcap API key is expired or no longer active.';
};

/**
 * Startup/global warning for an expired or inactive saved Hubcap key.
 *
 * Duplicated work (Store, Library, Workshop, version switch) publishes the
 * same verdict, so this modal is shown once per session and key instead of
 * beside every failed action.
 */
export const HubcapKeyExpiredModal = ({ reason, onGoToSettings, onClose }: HubcapKeyExpiredModalProps) => (
  <ModalShell
    title="Hubcap API Key expired"
    onClose={onClose}
    containerClassName="uninstall-modal"
    bodyClassName="uninstall-modal-body"
  >
    <p className="uninstall-modal-copy">{detailFor(reason)}</p>
    <p className="uninstall-modal-copy">
      Adding games, manifest generation, Workshop mods, and whatever else that need Hubcap will fail until you replace
      the key in Settings.
    </p>

    <div className="uninstall-modal-actions">
      <button type="button" className="uninstall-btn uninstall-btn-primary" onClick={onGoToSettings}>
        Settings
      </button>
      <button type="button" className="uninstall-btn uninstall-btn-secondary" onClick={onClose}>
        Later
      </button>
    </div>
  </ModalShell>
);
