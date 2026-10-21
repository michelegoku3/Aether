import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { VERSIONING_PROGRESS_EVENT, VersioningProgress } from '../constants/library';

/**
 * Subscribes to `versioning://progress` events emitted by the backend while a
 * build is being applied (or retried in the background ACF queue). Only keeps
 * the latest progress event for the requested `appId`; ignores updates for
 * other apps so a background retry can't stomp a foreground modal.
 *
 * Pass `active:false` to drop events when the UI is not interested (e.g. modal
 * closed) — the subscription is torn down automatically.
 */
export const useVersioningProgress = (appId: number, active: boolean) => {
  const [progress, setProgress] = useState<VersioningProgress | null>(null);

  useEffect(() => {
    if (!active) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    listen<VersioningProgress>(VERSIONING_PROGRESS_EVENT, (event) => {
      if (cancelled) return;
      const payload = event.payload;
      if (!payload || payload.appId !== appId) return;
      setProgress(payload);
    })
      .then((off) => {
        // A cleanup that ran before `listen` resolved cannot see `off`; release
        // the handler here so it does not outlive the component.
        if (cancelled) {
          off();
          return;
        }
        unlisten = off;
      })
      .catch((error) => console.warn('Versioning listener failed:', error));

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [appId, active]);

  return progress;
};
