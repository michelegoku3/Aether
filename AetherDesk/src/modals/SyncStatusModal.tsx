import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { ModalShell } from '../ui/ModalShell';
import { useVisiblePolling } from '../hooks/useVisiblePolling';
import { SYNC_STATUS_EVENT } from '../constants/library';
import { SYNC_LANES, type LaneStatus, type MonitorStatus } from '../types/sync';
import { toErrorMessage } from '../util/errors';

// Types live in `types/sync.ts` (IPC contract); re-exported here because
// historic consumers imported them from the modal.
export type { MonitorStatus } from '../types/sync';

export interface SyncStatusModalProps {
  /** X / Escape / overlay — dismiss. */
  onClose: () => void;
}

/** Recovery polling cadence while the popup is visible. The primary signal
 *  is the `sync://status-changed` event: polling only avoids going stale if
 *  an event is lost (same contract as the Library). */
const SYNC_POLL_INTERVAL_MS = 5_000;
/** Backoff while the window is hidden/minimized with the popup open. */
const SYNC_POLL_BACKOFF_MS = 20_000;

const formatEpoch = (epoch: number | null): string => {
  if (!epoch) return '—';
  return new Date(epoch * 1000).toLocaleTimeString();
};

const formatRetry = (epoch: number | null): string => {
  if (!epoch) return '—';
  const seconds = Math.max(0, Math.round(epoch - Date.now() / 1000));
  if (seconds <= 90) return `in ${seconds}s`;
  return `in ~${Math.round(seconds / 60)} min`;
};

const describeApp = (appId: number | null): string => (appId === null ? 'Workshop set' : `App ${appId}`);

const plural = (count: number, singular: string) => `${count} ${singular}${count === 1 ? '' : 's'}`;

const LaneSection = ({ title, hint, lane }: { title: string; hint: string; lane: LaneStatus }) => (
  <div className="uninstall-modal-copy">
    <strong>{title}</strong>
    <span style={{ display: 'block', opacity: 0.75, marginBottom: 4 }}>{hint}</span>
    <span style={{ display: 'block', marginBottom: 4 }}>
      Completed: {lane.processedCount} · Last run: {formatEpoch(lane.lastRunEpoch)}
    </span>
    {lane.lastError && (
      <span style={{ display: 'block', color: '#e06c75', marginBottom: 4 }}>Last error: {lane.lastError}</span>
    )}
    {(lane.unresolved?.length ?? 0) > 0 && (
      <span style={{ display: 'block', color: '#e5c07b', marginBottom: 4 }}>
        Gave up on {plural(lane.unresolved!.length, 'task')} — no longer retried; the next Steam-side change queues{' '}
        {lane.unresolved!.length === 1 ? 'it' : 'them'} again:
        {lane.unresolved!.map((task, index) => (
          <span key={`${task.appId ?? 'lane'}-unresolved-${index}`} style={{ display: 'block', paddingLeft: 12 }}>
            {describeApp(task.appId)} — after {plural(task.attempts, 'attempt')}
          </span>
        ))}
      </span>
    )}
    {lane.pending.length > 0 ? (
      <span style={{ display: 'block' }}>
        Queued ({lane.pending.length}):
        {lane.pending.map((task, index) => (
          <span key={`${task.appId ?? 'lane'}-${index}`} style={{ display: 'block', paddingLeft: 12 }}>
            {describeApp(task.appId)} — attempt {task.attempts + 1}, retry {formatRetry(task.nextRetryEpoch)}
          </span>
        ))}
      </span>
    ) : (
      <span style={{ display: 'block', opacity: 0.75 }}>Queue empty.</span>
    )}
  </div>
);

/**
 * Live status of the background Steam-change synchronizer (pin realignment,
 * Lua manifest repair, Workshop staging).
 *
 * Transport: **push first, polling as recovery**. The backend emits
 * `sync://status-changed` only when the snapshot really changes, so the popup
 * is instant and never polls IPC when there is nothing new; the slow poll
 * (with hidden-window backoff, hook shared with LogView) stays as a safety
 * net in case an event is lost.
 */
export const SyncStatusModal = ({ onClose }: SyncStatusModalProps) => {
  const [status, setStatus] = useState<MonitorStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<MonitorStatus>('get_hubcap_monitor_status'));
      setError(null);
    } catch (err: unknown) {
      setError(toErrorMessage(err));
    }
  }, []);

  // Push: every real snapshot change arrives here.
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    listen<MonitorStatus>(SYNC_STATUS_EVENT, (event) => {
      if (event.payload) {
        setStatus(event.payload);
        setError(null);
      }
    })
      .then((off) => {
        if (cancelled) {
          off();
          return;
        }
        unlisten = off;
      })
      .catch(() => {
        // No event bridge available: recovery polling remains.
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Recovery: immediate tick on open, then slow cadence.
  useVisiblePolling(refresh, {
    intervalMs: SYNC_POLL_INTERVAL_MS,
    backoffMs: SYNC_POLL_BACKOFF_MS,
  });

  return (
    <ModalShell
      title="Background sync"
      onClose={onClose}
      containerClassName="uninstall-modal"
      bodyClassName="uninstall-modal-body"
    >
      {error && <p className="uninstall-modal-copy">Status unavailable: {error}</p>}
      {!status && !error && <p className="uninstall-modal-copy">Loading synchronizer state…</p>}
      {status && (
        <>
          <p className="uninstall-modal-copy">
            The synchronizer watches Steam (game updates, Lua changes, Workshop items) and reacts with local work only:
            pinned Lua rows are realigned to the manifests Steam installed, missing manifests are repaired from local
            backups first, and Workshop manifests are staged. Manifest generation on demand stays with AetherDLL — this
            monitor never downloads game packages.
          </p>
          <p className="uninstall-modal-copy">
            State: {status.running ? 'running' : 'stopped'} · Last scan: {formatEpoch(status.lastScanEpoch)} · Steam
            path: {status.steamPathConfigured ? 'configured' : 'missing'} · Hubcap key:{' '}
            {status.hubcapKeyConfigured ? 'configured' : 'missing'}
          </p>
          {SYNC_LANES.map((descriptor) => (
            <LaneSection
              key={descriptor.lane}
              title={descriptor.title}
              hint={descriptor.hint}
              lane={status[descriptor.lane]}
            />
          ))}
        </>
      )}
      <div className="uninstall-modal-actions">
        <button type="button" className="uninstall-btn uninstall-btn-secondary" onClick={() => void refresh()}>
          Refresh now
        </button>
      </div>
    </ModalShell>
  );
};
