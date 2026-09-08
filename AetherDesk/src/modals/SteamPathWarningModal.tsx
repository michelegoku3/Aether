import { ModalShell } from '../ui/ModalShell';

export interface SteamPathWarningModalProps {
  /** User pressed "Go to Settings" — caller switches tab and closes. */
  onGoToSettings: () => void;
  /** X / Escape / overlay / "Later" — dismiss without changing anything. */
  onClose: () => void;
}

/**
 * No-Steam-path warning, in the same style as the OST first-enable popup.
 *
 * Shown on startup and whenever the user leaves the Settings tab (or tries to
 * save) without a valid stored Steam path. Unlike the OST warning this is NOT
 * a one-time ack: it reappears until a valid path is configured, because every
 * Steam-dependent feature needs it.
 */
export const SteamPathWarningModal = ({ onGoToSettings, onClose }: SteamPathWarningModalProps) => (
  <ModalShell
    title="Steam path not found"
    onClose={onClose}
    containerClassName="uninstall-modal"
    bodyClassName="uninstall-modal-body"
  >
    <p className="uninstall-modal-copy">
      No valid Steam installation path was found. AetherDesk needs to know
      where Steam is installed.
    </p>
    <p className="uninstall-modal-copy">
      Go to Settings and set your Steam installation path (or use Auto Detect),
      then restart AetherDesk and Steam to be safe.
    </p>

    <div className="uninstall-modal-actions">
      <button
        type="button"
        className="uninstall-btn uninstall-btn-primary"
        onClick={onGoToSettings}
      >
        Go to Settings
      </button>
      <button
        type="button"
        className="uninstall-btn uninstall-btn-secondary"
        onClick={onClose}
      >
        Later
      </button>
    </div>
  </ModalShell>
);
