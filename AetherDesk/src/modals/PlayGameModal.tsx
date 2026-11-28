import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ModalShell } from '../ui/ModalShell';
import { toErrorMessage } from '../util/errors';
import {
  formatGb,
  formatMb,
  RAM_DEFAULT_MIN_MB,
  RAM_THRESHOLD_OPTIONS,
  type PlayGameReport,
  type RamOptimizeReport,
  type RamPreview,
  type RamProcessEntry,
  type SystemMemoryInfo,
} from '../types/play';

interface PlayGameModalProps {
  game: { appId: string; name: string; installed?: boolean; installing?: boolean };
  onClose: () => void;
}

interface ProcessGroup {
  key: string;
  name: string;
  pids: number[];
  memoryKb: number;
}

const KEEP_APPS_STORAGE_KEY = 'aether.play_keep_apps';
const MIN_MB_STORAGE_KEY = 'aether.play_min_mb';
const EMPTY_MEMORY: SystemMemoryInfo = { totalKb: 0, usedKb: 0, availableKb: 0 };

const readKeepApps = (): Set<string> => {
  try {
    const raw = localStorage.getItem(KEEP_APPS_STORAGE_KEY);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set();
    return new Set(parsed.filter((item): item is string => typeof item === 'string').map((s) => s.toLowerCase()));
  } catch {
    return new Set();
  }
};

const saveKeepApps = (apps: Set<string>) => {
  try {
    localStorage.setItem(KEEP_APPS_STORAGE_KEY, JSON.stringify(Array.from(apps).sort()));
  } catch {
    // ignore storage errors
  }
};

const readStoredMinMb = (): number => {
  try {
    const raw = Number(localStorage.getItem(MIN_MB_STORAGE_KEY));
    if (RAM_THRESHOLD_OPTIONS.includes(raw as (typeof RAM_THRESHOLD_OPTIONS)[number])) {
      return raw;
    }
  } catch {
    // ignore storage errors
  }
  return RAM_DEFAULT_MIN_MB;
};

const normalizePreview = (raw: RamPreview | RamProcessEntry[]): RamPreview => {
  if (Array.isArray(raw)) {
    return { entries: raw, memory: EMPTY_MEMORY };
  }
  return {
    entries: raw?.entries ?? [],
    memory: raw?.memory ?? EMPTY_MEMORY,
  };
};

const summarizeNames = (names: string[]): Array<{ name: string; count: number }> => {
  const counts = new Map<string, number>();
  const order: string[] = [];
  for (const name of names) {
    if (!counts.has(name)) {
      order.push(name);
      counts.set(name, 1);
    } else {
      counts.set(name, (counts.get(name) ?? 0) + 1);
    }
  }
  return order.map((name) => ({ name, count: counts.get(name) ?? 1 }));
};

/**
 * Pre-launch gate for "Play".
 *
 * The backend decides which processes may be closed; this dialog groups
 * multi-process apps, shows system RAM impact, lets the user adjust threshold
 * or keep specific apps across launches, and asks before signalling anything.
 */
export const PlayGameModal = ({ game, onClose }: PlayGameModalProps) => {
  const [minMemoryMb, setMinMemoryMb] = useState<number>(readStoredMinMb);
  const [refreshTick, setRefreshTick] = useState(0);
  const [preview, setPreview] = useState<RamPreview | null>(null);
  const [isPreviewLoading, setIsPreviewLoading] = useState(true);
  const [excludedApps, setExcludedApps] = useState<Set<string>>(readKeepApps);
  const [showKept, setShowKept] = useState(false);
  const [error, setError] = useState<string>('');
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<PlayGameReport | null>(null);
  const [launchedDirectly, setLaunchedDirectly] = useState(false);

  const notInstalled = game.installed === false || game.installing === true;

  useEffect(() => {
    if (notInstalled) return;
    let active = true;
    setIsPreviewLoading(true);
    invoke<RamPreview | RamProcessEntry[]>('preview_ram_optimizer', { minMemoryMb })
      .then((raw) => {
        if (!active) return;
        setPreview(normalizePreview(raw));
        setError('');
      })
      .catch((err: unknown) => {
        if (active) setError(toErrorMessage(err));
      })
      .finally(() => {
        if (active) setIsPreviewLoading(false);
      });
    return () => {
      active = false;
    };
  }, [minMemoryMb, notInstalled, refreshTick]);

  const entries = preview?.entries ?? null;
  const memory = preview?.memory ?? EMPTY_MEMORY;

  const closeCandidates = useMemo(() => (entries ?? []).filter((entry) => entry.verdict === 'close'), [entries]);
  const kept = useMemo(() => (entries ?? []).filter((entry) => entry.verdict === 'keep'), [entries]);

  const closeGroups = useMemo<ProcessGroup[]>(() => {
    const map = new Map<string, ProcessGroup>();
    for (const entry of closeCandidates) {
      const key = entry.name.toLowerCase();
      const existing = map.get(key);
      if (existing) {
        existing.pids.push(entry.pid);
        existing.memoryKb += entry.memoryKb;
      } else {
        map.set(key, {
          key,
          name: entry.name,
          pids: [entry.pid],
          memoryKb: entry.memoryKb,
        });
      }
    }
    return Array.from(map.values()).sort((a, b) => b.memoryKb - a.memoryKb);
  }, [closeCandidates]);

  const selectedGroups = useMemo(
    () => closeGroups.filter((group) => !excludedApps.has(group.key)),
    [closeGroups, excludedApps]
  );
  const selectedCandidates = useMemo(
    () => closeCandidates.filter((entry) => !excludedApps.has(entry.name.toLowerCase())),
    [closeCandidates, excludedApps]
  );
  const excludedPids = useMemo(
    () => closeCandidates.filter((entry) => excludedApps.has(entry.name.toLowerCase())).map((entry) => entry.pid),
    [closeCandidates, excludedApps]
  );
  const selectedCloseKb = selectedGroups.reduce((sum, group) => sum + group.memoryKb, 0);

  const toggleApp = (appKey: string) => {
    setExcludedApps((prev) => {
      const next = new Set(prev);
      if (next.has(appKey)) {
        next.delete(appKey);
      } else {
        next.add(appKey);
      }
      saveKeepApps(next);
      return next;
    });
  };

  const selectAllGroups = () => {
    const next = new Set(excludedApps);
    for (const group of closeGroups) {
      next.delete(group.key);
    }
    saveKeepApps(next);
    setExcludedApps(next);
  };

  const selectNoneGroups = () => {
    const next = new Set(excludedApps);
    for (const group of closeGroups) {
      next.add(group.key);
    }
    saveKeepApps(next);
    setExcludedApps(next);
  };

  const handleThresholdChange = (nextMb: number) => {
    setMinMemoryMb(nextMb);
    try {
      localStorage.setItem(MIN_MB_STORAGE_KEY, String(nextMb));
    } catch {
      // ignore storage errors
    }
  };

  const launch = async (skipOptimize: boolean) => {
    setBusy(true);
    setError('');
    setLaunchedDirectly(skipOptimize);
    try {
      const report = await invoke<PlayGameReport>('optimize_and_launch_game', {
        appId: Number(game.appId),
        minMemoryMb,
        skipOptimize,
        excludePids: skipOptimize ? [] : excludedPids,
      });
      setResult(report);
    } catch (err: unknown) {
      setError(toErrorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const report: RamOptimizeReport | null = result?.report ?? null;
  const closedSummary = useMemo(() => summarizeNames(report?.closed ?? []), [report]);
  const usedPct = memory.totalKb > 0 ? Math.min(100, (memory.usedKb / memory.totalKb) * 100) : 0;
  const reclaimPct = memory.totalKb > 0 ? Math.min(usedPct, (selectedCloseKb / memory.totalKb) * 100) : 0;

  return (
    <ModalShell
      title={`Play ${game.name}`}
      onClose={onClose}
      containerClassName="play-modal"
      bodyClassName="play-modal-body"
    >
      {notInstalled ? (
        <>
          <p className="uninstall-modal-lead">This game is not installed yet.</p>
          <p className="uninstall-modal-copy">
            {game.installing
              ? 'Steam is still writing its content. Wait for the install to finish, then play it.'
              : 'Install it from Steam first; the Play button only starts games with local content.'}
          </p>
          <div className="uninstall-modal-actions">
            <button type="button" className="uninstall-btn uninstall-btn-secondary" onClick={onClose}>
              Close
            </button>
          </div>
        </>
      ) : result ? (
        <>
          <p className="uninstall-modal-lead">{result.message}</p>
          {launchedDirectly ? (
            <p className="uninstall-modal-copy">Launched directly without closing background applications.</p>
          ) : (
            report && (
              <>
                <p className="uninstall-modal-copy">
                  Closed {report.closed.length} process(es) and reclaimed about {formatMb(report.reclaimedKb)}.
                </p>
                {closedSummary.length > 0 && (
                  <ul className="play-process-list">
                    {closedSummary.map((item) => (
                      <li key={item.name}>
                        <span className="play-process-name">{item.name}</span>
                        {item.count > 1 && <span className="play-process-meta">×{item.count}</span>}
                      </li>
                    ))}
                  </ul>
                )}
                {report.failed.length > 0 && (
                  <p className="uninstall-modal-copy">Could not close: {report.failed.join(', ')}.</p>
                )}
                {report.gone.length > 0 && (
                  <p className="uninstall-modal-copy">Already closed by themselves: {report.gone.join(', ')}.</p>
                )}
              </>
            )
          )}
          <div className="uninstall-modal-actions">
            <button type="button" className="uninstall-btn uninstall-btn-secondary" onClick={onClose}>
              Done
            </button>
          </div>
        </>
      ) : (
        <>
          <p className="uninstall-modal-lead">
            Before launching, AetherDesk can close memory-hungry applications to free RAM.
          </p>
          <p className="uninstall-modal-copy">
            Steam, AetherDesk, anti-cheat, overlays, other launchers and every running game are never closed. Uncheck
            any application you want to keep open, or launch directly without closing anything.
          </p>

          {memory.totalKb > 0 && (
            <div className="play-ram-overview">
              <div className="play-ram-summary">
                <span>
                  System RAM: <strong>{formatGb(memory.usedKb)}</strong> / {formatGb(memory.totalKb)} used (
                  {formatGb(memory.availableKb)} free)
                </span>
                {selectedCloseKb > 0 && (
                  <span className="play-ram-reclaim-text">+{formatMb(selectedCloseKb)} reclaimable</span>
                )}
              </div>
              <div className="play-ram-bar" aria-hidden="true">
                <div className="play-ram-bar-used" style={{ width: `${usedPct.toFixed(1)}%` }}>
                  {reclaimPct > 0 && (
                    <div
                      className="play-ram-bar-reclaim"
                      style={{ width: `${Math.min(100, (reclaimPct / Math.max(usedPct, 0.1)) * 100).toFixed(1)}%` }}
                    />
                  )}
                </div>
              </div>
            </div>
          )}

          <div className="play-toolbar">
            <label className="play-threshold-label">
              Min RAM:{' '}
              <select
                className="settings-select"
                value={minMemoryMb}
                disabled={busy || isPreviewLoading}
                onChange={(e) => handleThresholdChange(Number(e.target.value))}
              >
                {RAM_THRESHOLD_OPTIONS.map((mb) => (
                  <option key={mb} value={mb}>
                    {mb} MB
                  </option>
                ))}
              </select>
            </label>
            <button
              type="button"
              className="settings-small-btn"
              disabled={busy || isPreviewLoading}
              onClick={() => setRefreshTick((tick) => tick + 1)}
            >
              {isPreviewLoading ? 'Scanning…' : 'Refresh'}
            </button>
            {closeGroups.length > 0 && (
              <>
                <button type="button" className="settings-small-btn" disabled={busy} onClick={selectAllGroups}>
                  Select all
                </button>
                <button type="button" className="settings-small-btn" disabled={busy} onClick={selectNoneGroups}>
                  Select none
                </button>
              </>
            )}
          </div>

          {error && <p className="uninstall-modal-copy play-modal-error">{error}</p>}

          {entries === null && !error ? (
            <p className="uninstall-modal-copy">Reading the process list…</p>
          ) : (
            <>
              <p className="uninstall-modal-copy">
                Selected to close: <strong>{selectedGroups.length}</strong> / {closeGroups.length} app(s) (
                {selectedCandidates.length} proc(s), {formatMb(selectedCloseKb)}) &nbsp;·&nbsp;kept:{' '}
                <strong>{kept.length}</strong>
              </p>
              {closeGroups.length > 0 && (
                <ul className="play-process-list">
                  {closeGroups.slice(0, 40).map((group) => {
                    const checked = !excludedApps.has(group.key);
                    return (
                      <li key={group.key}>
                        <label className="play-process-check">
                          <input
                            type="checkbox"
                            checked={checked}
                            disabled={busy}
                            onChange={() => toggleApp(group.key)}
                          />
                          <span className="play-process-name">{group.name}</span>
                          {group.pids.length > 1 && (
                            <span className="play-process-count">{group.pids.length} procs</span>
                          )}
                        </label>
                        <span className="play-process-meta">
                          {group.pids.length > 1
                            ? formatMb(group.memoryKb)
                            : `${formatMb(group.memoryKb)} · PID ${group.pids[0]}`}
                        </span>
                      </li>
                    );
                  })}
                  {closeGroups.length > 40 && (
                    <li className="play-process-more">…and {closeGroups.length - 40} more apps</li>
                  )}
                </ul>
              )}
              {kept.length > 0 && (
                <div className="play-kept-details">
                  <button type="button" className="play-kept-summary" onClick={() => setShowKept((prev) => !prev)}>
                    {showKept ? '▾' : '▸'} Protected / Kept ({kept.length})
                  </button>
                  {showKept && (
                    <ul className="play-process-list play-kept-list">
                      {kept.slice(0, 30).map((entry) => (
                        <li key={entry.pid}>
                          <span className="play-process-name">{entry.name}</span>
                          <span className="play-process-meta">
                            {formatMb(entry.memoryKb)} · {entry.reason}
                          </span>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
              )}
            </>
          )}

          <div className="uninstall-modal-actions">
            <button
              type="button"
              className="uninstall-btn uninstall-btn-danger"
              disabled={busy || selectedCandidates.length === 0}
              onClick={() => void launch(false)}
            >
              {busy ? 'Launching…' : 'Optimize & Play'}
            </button>
            <button
              type="button"
              className="uninstall-btn uninstall-btn-primary"
              disabled={busy}
              onClick={() => void launch(true)}
            >
              {busy ? 'Launching…' : 'Play Directly'}
            </button>
            <button type="button" className="uninstall-btn uninstall-btn-secondary" disabled={busy} onClick={onClose}>
              Cancel
            </button>
          </div>
        </>
      )}
    </ModalShell>
  );
};
