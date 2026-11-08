import { ModalShell } from '../ui/ModalShell';

export interface SteamCloseConfirmModalProps {
  /** User accepted closing Steam. The caller runs the operation with the kill allowed. */
  onConfirm: () => void;
  /** X / Escape / overlay — dismiss and abort the whole operation. */
  onCancel: () => void;
  /**
   * Optional NO handler, for flows where refusing the Steam close still has a
   * meaning of its own (the AetherDesk uninstall skips the Steam clean and
   * keeps uninstalling). Without it NO is a plain dismissal.
   */
  onReject?: () => void;
}

/**
 * Gate for the Steam-side operations (AetherDLL install/update/uninstall, Reset
 * Path, the uninstall Steam clean) that cannot run while Steam holds its files
 * open. YES lets the backend terminate Steam completely and proceed; NO only
 * closes the dialog. Steam is never restarted afterwards.
 *
 * The dialog owns no progress state: the caller dismisses it before starting
 * the operation, so the panel behind it carries the locked "working" state.
 */
export const SteamCloseConfirmModal = ({ onConfirm, onCancel, onReject }: SteamCloseConfirmModalProps) => (
  <ModalShell
    title="Close Steam?"
    onClose={onCancel}
    containerClassName="uninstall-modal"
    bodyClassName="uninstall-modal-body"
  >
    <p className="uninstall-modal-lead">This operation will close Steam. Are you sure you want to do it?</p>
    <p className="uninstall-modal-copy">If you have a game open, save the game and close it.</p>

    <div className="uninstall-modal-actions">
      <button type="button" className="uninstall-btn uninstall-btn-danger" onClick={onConfirm}>
        Yes
      </button>
      <button type="button" className="uninstall-btn uninstall-btn-secondary" onClick={onReject ?? onCancel}>
        No
      </button>
    </div>
  </ModalShell>
);
