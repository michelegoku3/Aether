import { useCallback, useEffect, useRef } from 'react';

/**
 * Single watchdog timer for long-running async operations.
 *
 * Both version editors (Auto apply and Manual edits) used to duplicate the
 * same ref + clear + unmount-cleanup + arm logic; this hook centralizes it
 * (DRY). The armed callback fires once after `ms`; arming again cancels the
 * previous timer, and the timer is always cleared on unmount.
 *
 * @example
 *   const { arm, clear } = useWatchdog();
 *   arm(() => setStatus({...}), 90_000);  // in the async handler
 *   clear();                               // on success/failure
 */
export const useWatchdog = () => {
  const ref = useRef<number | null>(null);

  const clear = useCallback(() => {
    if (ref.current !== null) {
      window.clearTimeout(ref.current);
      ref.current = null;
    }
  }, []);

  // Never leave a pending timer behind when the component unmounts.
  useEffect(() => clear, [clear]);

  const arm = useCallback(
    (callback: () => void, ms: number) => {
      clear();
      ref.current = window.setTimeout(callback, ms);
    },
    [clear],
  );

  return { arm, clear };
};
