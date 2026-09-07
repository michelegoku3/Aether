import { ModalShell } from '../ui/ModalShell';

export interface OstWarningModalProps {
  /** User pressed "I understand" — caller persists the ack and enables OST. */
  onConfirm: () => void;
  /** X / Escape / overlay — abort without changing anything. */
  onCancel: () => void;
}

/**
 * First-enable warning for the OST pattern source opt-in.
 *
 * Shown once when the user flips the "Use OST pattern source" switch ON for
 * the first time. Dismissing via X/Escape/overlay calls `onCancel` without
 * persisting anything, so the switch stays OFF and the popup reappears on the
 * next attempt. Only `onConfirm` ("I understand") lets the caller enable OST.
 */
export const OstWarningModal = ({ onConfirm, onCancel }: OstWarningModalProps) => (
  <ModalShell
    title="OST pattern source"
    onClose={onCancel}
    containerClassName="uninstall-modal"
    bodyClassName="uninstall-modal-body"
  >
    <p className="uninstall-modal-lead">
      Aether needs up-to-date patterns for every new Steam version. If Steam
      updates while updates are unlocked and no fresh patterns are available,
      games added with Aether will show Buy instead of Play or Install.
    </p>
    <p className="uninstall-modal-copy">
      There are two pattern sources: the Aether developer feed (updated within
      about 1 hour of a Steam update) and OST (usually available within
      minutes). With OST enabled, OST patterns are used right after a Steam
      update, then replaced by the developer feed as soon as it is ready.
    </p>
    <p className="uninstall-modal-copy">
      OST patterns lack cloud patterns, which can lead to save-game loss (not
      tested with Aether). Enable at your own risk.
    </p>

    <div className="uninstall-modal-actions">
      <button
        type="button"
        className="uninstall-btn uninstall-btn-primary"
        onClick={onConfirm}
      >
        I understand
      </button>
    </div>
  </ModalShell>
);
