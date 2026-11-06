import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { DESK_UPDATE_PROGRESS_EVENT, DeskUpdatePhase, DeskUpdateProgress } from '../constants/updates';

export interface DeskUpdateState {
  phase: DeskUpdatePhase;
  /** 0..100 across the whole update, not just the download. */
  progress: number;
  /** Seconds since the first progress event, so a stalled update is visible. */
  elapsedSeconds: number;
}

/** Where each phase sits on the one bar the panel shows. */
const PHASE_PROGRESS: Record<DeskUpdatePhase, number> = {
  'looking-up': 3,
  // The download fills the band from its start to `DOWNLOAD_END` by bytes.
  downloading: 3,
  extracting: 78,
  staging: 92,
  restarting: 98,
};

const DOWNLOAD_END = 75;

/**
 * Overall bar value for one event.
 *
 * The download is the only phase with a measurable size, so it gets the largest
 * band (3-75) and every other phase is an anchor past it: the bar never jumps
 * backwards and never sits still for the whole extraction or restart.
 */
const overallProgress = ({ phase, downloadedBytes, totalBytes }: DeskUpdateProgress): number => {
  if (phase !== 'downloading') return PHASE_PROGRESS[phase];
  if (totalBytes === null || totalBytes <= 0) return PHASE_PROGRESS.downloading;

  const ratio = Math.min(1, Math.max(0, downloadedBytes / totalBytes));
  const band = DOWNLOAD_END - PHASE_PROGRESS.downloading;
  return Math.round(PHASE_PROGRESS.downloading + band * ratio);
};

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

  const elapsedSeconds = startedAt === null ? 0 : Math.max(0, Math.floor((now - startedAt) / 1000));

  return {
    phase: progress.phase,
    progress: overallProgress(progress),
    elapsedSeconds,
  };
};
