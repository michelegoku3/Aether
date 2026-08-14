import React, { useEffect } from 'react';

export interface ModalShellProps {
  /** Dialog title shown in the header. */
  title: React.ReactNode;
  /** Called when the user dismisses via X, Escape, or overlay click. */
  onClose: () => void;
  /** When true, dismiss gestures are ignored (e.g. while a command runs). */
  closeDisabled?: boolean;
  /** Extra class names for the container (size variants, etc.). */
  containerClassName?: string;
  /** Optional extra class on the body section. */
  bodyClassName?: string;
  children: React.ReactNode;
}

/**
 * Shared modal chrome: overlay, header with ×, Escape, and click-outside.
 * Domain modals own their body content and action buttons — this shell only
 * handles presentation and dismiss behaviour (DRY across confirm dialogs).
 */
export const ModalShell = ({
  title,
  onClose,
  closeDisabled = false,
  containerClassName = '',
  bodyClassName = '',
  children,
}: ModalShellProps) => {
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !closeDisabled) {
        event.preventDefault();
        onClose();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [closeDisabled, onClose]);

  const requestClose = () => {
    if (!closeDisabled) onClose();
  };

  return (
    <div
      className="modal-overlay"
      onClick={requestClose}
      role="presentation"
    >
      <div
        className={`modal-container ${containerClassName}`.trim()}
        onClick={(event) => event.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <div className="modal-header">
          <span className="modal-title">{title}</span>
          <button
            type="button"
            className="modal-close-btn"
            onClick={requestClose}
            disabled={closeDisabled}
            aria-label="Close"
            title="Close"
          >
            &times;
          </button>
        </div>

        <div className="modal-separator" />

        <div className={`modal-body ${bodyClassName}`.trim()}>
          {children}
        </div>
      </div>
    </div>
  );
};
