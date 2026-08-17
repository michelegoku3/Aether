import { useEffect } from 'react';

/**
 * Shared modal dismiss behaviour: pressing ESC closes the popup.
 *
 * Centralizes the keydown handling so every popup behaves identically (DRY).
 * For click-outside dismissal, pair it with an `onClick` on the modal overlay
 * (guarded by the same `disabled` flag) and `stopPropagation` on the container.
 *
 * @param onClose  Called when ESC is pressed (and not disabled).
 * @param disabled When true, ESC is ignored (e.g. while an operation runs).
 */
export const useModalDismiss = (onClose: () => void, disabled = false) => {
  useEffect(() => {
    if (disabled) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [onClose, disabled]);
};
