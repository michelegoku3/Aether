import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { DESK_UPDATE_PROGRESS_EVENT, DeskUpdatePhase, DeskUpdateProgress } from '../constants/updates';

export interface DeskUpdateState {
  phase: DeskUpdatePhase;
  /** 0..100 while downloading with a known size, `null` otherwise. */
  percent: number | null;
  /** Seconds since the first progress event, so a stalled update is visible. */
  elapsedSeconds: number;
}

/**
 * Subscribes to `desk-update://progress` events emitted by
 * `commands/aether_desk.rs` while the portable update runs.
 *
 * The backend only emits on phase changes and every ~200 KB, so elapsed time
 * is counted here: a download that stops moving still shows the seconds
 * climbing, which is what tells the user apart "slow" from "stuck".
 *
 * Pass `active:false` when no update is running: the subscription is torn down
 * and the state resets, so a previous run cannot leak into the next.
 */
export const useDeskUpdateProgress = (active: boolean): DeskUpdateState | null => {
  const [progress, setProgress] = useState<DeskUpdateProgress | null>(null);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [now, setNow] = useState(0);

  useEffect(() => {
    if (!active) {
      setProgress(null);
      setStartedAt(null);
      return;
    }

    let unlisten: (() => void) | undefined;
    let cancelled = false;

    listen<DeskUpdateProgress>(DESK_UPDATE_PROGRESS_EVENT, (event) => {
      if (cancelled) return;
      const payload = event.payload;
      if (!payload) return;
      setStartedAt((previous) => previous ?? Date.now());
      setNow(Date.now());
      setProgress(payload);
    })
      .then((off) => {
        // A cleanup that ran before `listen` resolved cannot see `off`; release
        // the handler here so it does not outlive the view.
        if (cancelled) {
          off();
          return;
        }
        unlisten = off;
      })
      .catch((error) => console.warn('Desk update listener failed:', error));

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [active]);

  // Ticks the elapsed counter once a second; the download itself may be silent
  // for longer than that between two events.
  useEffect(() => {
    if (!active || startedAt === null) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [active, startedAt]);

  if (!progress) return null;

  const total = progress.totalBytes;
  const percent =
    total !== null && total > 0 ? Math.min(100, Math.round((progress.downloadedBytes / total) * 100)) : null;
  const elapsedSeconds = startedAt === null ? 0 : Math.max(0, Math.floor((now - startedAt) / 1000));

  return { phase: progress.phase, percent, elapsedSeconds };
};
