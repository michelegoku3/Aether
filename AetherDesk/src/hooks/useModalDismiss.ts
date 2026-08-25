import { useCallback, useEffect } from 'react';

/**
 * Shared modal dismiss behaviour: pressing ESC closes the popup.
 *
 * Centralizes the keydown handling so every popup behaves identically (DRY).
 * For click-outside dismissal, pair it with `useOverlayDismiss` below.
 *
 * @param onClose  Called when ESC is pressed (and not disabled).
 * @param disabled When true, ESC is ignored (e.g. while an operation runs,
 *                 or while a CHILD popup is open so only the child closes —
 *                 see LibraryGameActionsModal for the stacked-popup chain).
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

/**
 * Shared click-outside dismiss handler for a modal overlay.
 *
 * Returns a stable onClick callback that closes the popup unless disabled.
 * The built-in `stopPropagation` is the important part: overlays of NESTED
 * popups (e.g. OnlineChoiceModal inside the Modify popup's overlay) live in
 * the DOM inside their parent overlay — without the stop the click would
 * bubble up and close the whole chain instead of just the top popup. For
 * top-level popups the stop is harmless.
 *
 * Pair with `useModalDismiss` (ESC) and the same `disabled` flag.
 */
export const useOverlayDismiss = (onClose: () => void, disabled = false) =>
  useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      if (disabled) return;
      // Never let an overlay click reach a parent popup's overlay.
      event.stopPropagation();
      onClose();
    },
    [onClose, disabled],
  );
