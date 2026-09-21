import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { ModalShell } from '../ui/ModalShell';
import { useVisiblePolling } from '../hooks/useVisiblePolling';
import { SYNC_STATUS_EVENT } from '../constants/library';
import { SYNC_LANES, type LaneStatus, type MonitorStatus } from '../types/sync';

// I tipi vivono in `types/sync.ts` (contratto IPC); riesportati qui perché i
// consumatori storici li importavano dal modale.
export type { LaneStatus, MonitorStatus, PendingTaskInfo } from '../types/sync';

export interface SyncStatusModalProps {
  /** X / Escape / overlay — dismiss. */
  onClose: () => void;
}

/** Cadenza del polling di recovery mentre il popup è visibile. Il segnale
 *  primario è l'evento `sync://status-changed`: il polling serve solo a non
 *  restare stale se un evento si perde (stesso contratto della Library). */
const SYNC_POLL_INTERVAL_MS = 5_000;
/** Backoff quando la finestra è nascosta/minimizzata col popup aperto. */
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

const describeApp = (appId: number | null): string =>
  appId === null ? 'Workshop set' : `App ${appId}`;

const plural = (count: number, singular: string) => `${count} ${singular}${count === 1 ? '' : 's'}`;

const LaneSection = ({ title, hint, lane }: { title: string; hint: string; lane: LaneStatus }) => (
  <div className="uninstall-modal-copy">
    <strong>{title}</strong>
    <span style={{ display: 'block', opacity: 0.75, marginBottom: 4 }}>{hint}</span>
    <span style={{ display: 'block', marginBottom: 4 }}>
      Completed: {lane.processedCount} · Last run: {formatEpoch(lane.lastRunEpoch)}
    </span>
    {lane.lastError && (
      <span style={{ display: 'block', color: '#e06c75', marginBottom: 4 }}>
        Last error: {lane.lastError}
      </span>
    )}
    {(lane.unresolved?.length ?? 0) > 0 && (
      <span style={{ display: 'block', color: '#e5c07b', marginBottom: 4 }}>
        Gave up on {plural(lane.unresolved!.length, 'task')} — no longer retried; the next
        Steam-side change queues {lane.unresolved!.length === 1 ? 'it' : 'them'} again:
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
            {describeApp(task.appId)} — attempt {task.attempts + 1}, retry{' '}
            {formatRetry(task.nextRetryEpoch)}
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
 * Trasporto: **push prima, polling come recovery**. Il backend emette
 * `sync://status-changed` solo quando lo snapshot cambia davvero, quindi il
 * popup è istantaneo e non interroga IPC quando non c'è nulla di nuovo; il
 * poll lento (con backoff a finestra nascosta, hook condiviso con LogView)
 * resta come rete di sicurezza se un evento si perde.
 */
export const SyncStatusModal = ({ onClose }: SyncStatusModalProps) => {
  const [status, setStatus] = useState<MonitorStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<MonitorStatus>('get_hubcap_monitor_status'));
      setError(null);
    } catch (err: any) {
      setError(String(err));
    }
  }, []);

  // Push: ogni cambiamento reale dello snapshot arriva qui.
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
        // Il bridge eventi non è disponibile: resta il polling di recovery.
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Recovery: tick immediato all'apertura, poi cadenza lenta.
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
            The synchronizer watches Steam (game updates, Lua changes, Workshop
            items) and reacts with local work only: pinned Lua rows are
            realigned to the manifests Steam installed, missing manifests are
            repaired from local backups first, and Workshop manifests are
            staged. Manifest generation on demand stays with AetherDLL — this
            monitor never downloads game packages.
          </p>
          <p className="uninstall-modal-copy">
            State: {status.running ? 'running' : 'stopped'} · Last scan:{' '}
            {formatEpoch(status.lastScanEpoch)} · Steam path:{' '}
            {status.steamPathConfigured ? 'configured' : 'missing'} · Hubcap key:{' '}
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
        <button
          type="button"
          className="uninstall-btn uninstall-btn-secondary"
          onClick={() => void refresh()}
        >
          Refresh now
        </button>
      </div>
    </ModalShell>
  );
};
