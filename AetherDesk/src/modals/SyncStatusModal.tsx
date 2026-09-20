import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ModalShell } from '../ui/ModalShell';

/** Live state of one synchronizer lane, mirrored from the Rust snapshot. */
interface LaneStatus {
  pending: { appId: number | null; attempts: number; nextRetryEpoch: number | null }[];
  /** Tasks dropped after the bounded retry ladder: no longer auto-retried. */
  unresolved?: { appId: number | null; attempts: number; nextRetryEpoch: number | null }[];
  processedCount: number;
  lastRunEpoch: number | null;
  lastError: string | null;
}

export interface MonitorStatus {
  running: boolean;
  startedEpoch: number | null;
  lastScanEpoch: number | null;
  steamPathConfigured: boolean;
  hubcapKeyConfigured: boolean;
  checkpointInitialized: boolean;
  pinSync: LaneStatus;
  pinRefresh: LaneStatus;
  repair: LaneStatus;
  workshop: LaneStatus;
}

export interface SyncStatusModalProps {
  /** X / Escape / overlay — dismiss. */
  onClose: () => void;
}

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
        Given up on ({lane.unresolved!.length}) — no longer retried; the next Steam-side
        change queues {lane.unresolved!.length === 1 ? 'it' : 'them'} again:
        {lane.unresolved!.map((task, index) => (
          <span key={`${task.appId ?? 'lane'}-unresolved-${index}`} style={{ display: 'block', paddingLeft: 12 }}>
            {describeApp(task.appId)} — after {task.attempts} attempt{task.attempts === 1 ? '' : 's'}
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
 * Lua manifest repair, Workshop staging), refreshed while open so the
 * exponential backoff of a failing task is never invisible.
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

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 5000);
    return () => window.clearInterval(timer);
  }, [refresh]);

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
          <LaneSection
            title="Pin sync (after Steam updates)"
            hint="Realigns commented pins of games with updates enabled and archives the new manifests."
            lane={status.pinSync}
          />
          <LaneSection
            title="Pin refresh (Hubcap contents diff)"
            hint="Periodically diffs Lua pins against the manifests Hubcap packages (free endpoint), stages missing manifests and realigns commented pins of updates-ON games."
            lane={status.pinRefresh}
          />
          <LaneSection
            title="Manifest repair (Lua changes)"
            hint="Local-first repair of a replaced or edited Lua. Requires a Hubcap key for manifests that are not on disk."
            lane={status.repair}
          />
          <LaneSection
            title="Workshop staging"
            hint="Stages missing Workshop item manifests when the installed Workshop set changes."
            lane={status.workshop}
          />
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
