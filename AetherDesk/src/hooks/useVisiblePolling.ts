import { useCallback, useEffect, useRef } from 'react';

export interface VisiblePollingOptions {
  /** Cadence while the document is visible. */
  intervalMs: number;
  /** Reduced cadence while the document is hidden (minimized window,
   *  another OS tab active). Polling never stops entirely: coming back to
   *  the view must not show overly stale data. */
  backoffMs: number;
  /** `false` stops the loop completely (e.g. closed panel). */
  enabled?: boolean;
  /** Runs one tick immediately on activation instead of waiting for the first
   *  interval. Default `true`. */
  immediate?: boolean;
  /** Changing this value restarts the loop (and, with `immediate`, runs a tick
   *  right away). For consumers whose query depends on UI input — e.g. the
   *  selected log source — which must reload data on change, not on the next
   *  tick. */
  resetKey?: unknown;
}

/**
 * Shared "visibility-aware" polling.
 *
 * Rules (the same ones `LogView` applied by hand and the audit asked to
 * extend to `SyncStatusModal`):
 *
 * 1. **Self-rescheduling setTimeout, never setInterval**: the next tick starts
 *    AFTER the call completes, so a slow IPC call cannot pile up concurrent
 *    requests.
 * 2. **Backoff while the document is hidden**: `backoffMs` instead of
 *    `intervalMs`, re-evaluated on every tick (the user may minimize at any
 *    moment).
 * 3. **Immediate resume when visible again**: `visibilitychange` → one tick
 *    right away, then the chain restarts.
 * 4. **A single chain at a time**: every chain has an id; an in-flight tick at
 *    cleanup (or `resetKey`) time cannot reschedule anything.
 *
 * `tick` is read through a ref: consumers may pass a function created on every
 * render (closing over their own state) without restarting the loop.
 */
export const useVisiblePolling = (
  tick: () => void | Promise<void>,
  { intervalMs, backoffMs, enabled = true, immediate = true, resetKey }: VisiblePollingOptions,
): void => {
  const tickRef = useRef(tick);

  // Updated after every render (not during: writing a ref during render is
  // unsafe with concurrent rendering).
  useEffect(() => {
    tickRef.current = tick;
  });

  const runOnce = useCallback(async () => {
    try {
      await tickRef.current();
    } catch {
      // The tick owns its error state (e.g. showing it in the UI). The
      // exception is swallowed here only to keep the chain alive.
    }
  }, []);

  useEffect(() => {
    if (!enabled) return;

    let timer = 0;
    let stopped = false;
    let chain = 0;

    // Function declarations (hoisted): `schedule` and `startChain` call each
    // other, and neither runs before both exist.
    function schedule(owner: number) {
      if (stopped || owner !== chain) return;
      const hidden = document.visibilityState !== 'visible';
      timer = window.setTimeout(() => startChain(), hidden ? backoffMs : intervalMs);
    }

    function startChain() {
      if (stopped) return;
      const owner = ++chain;
      void runOnce().then(() => schedule(owner));
    }

    const onVisibilityChange = () => {
      if (document.visibilityState !== 'visible') return;
      // Cancel the timer (possibly backing off) and restart right away: whoever
      // returns to the window must not wait out the backoff remainder.
      window.clearTimeout(timer);
      startChain();
    };

    if (immediate) {
      startChain();
    } else {
      const owner = ++chain;
      schedule(owner);
    }

    document.addEventListener('visibilitychange', onVisibilityChange);
    return () => {
      stopped = true;
      chain += 1;
      window.clearTimeout(timer);
      document.removeEventListener('visibilitychange', onVisibilityChange);
    };
    // `resetKey` is deliberately in the deps: it is how the consumer asks to
    // restart the loop (and reload data right away).
  }, [enabled, intervalMs, backoffMs, immediate, resetKey, runOnce]);
};
