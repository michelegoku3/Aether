import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ModalShell } from '../ui/ModalShell';
import { toErrorMessage } from '../util/errors';
import {
  formatMb,
  RAM_DEFAULT_MIN_MB,
  type PlayGameReport,
  type RamOptimizeReport,
  type RamProcessEntry,
} from '../types/play';

interface PlayGameModalProps {
  game: { appId: string; name: string; installed?: boolean; installing?: boolean };
  onClose: () => void;
}

/**
 * Pre-launch gate for "Play".
 *
 * The backend decides which processes may be closed; this dialog only shows
 * that plan and asks for consent, because closing applications can lose
 * unsaved work. The whitelist itself (Steam, anti-cheat, overlays, the game
 * folders) is described here and enforced server-side.
 */
export const PlayGameModal = ({ game, onClose }: PlayGameModalProps) => {
  const [entries, setEntries] = useState<RamProcessEntry[] | null>(null);
  const [error, setError] = useState<string>('');
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<PlayGameReport | null>(null);

  useEffect(() => {
    let active = true;
    invoke<RamProcessEntry[]>('preview_ram_optimizer', { minMemoryMb: RAM_DEFAULT_MIN_MB })
      .then((plan) => {
        if (active) setEntries(plan);
      })
      .catch((err: unknown) => {
        if (active) setError(toErrorMessage(err));
      });
    return () => {
      active = false;
    };
  }, []);

  const closeCandidates = useMemo(() => (entries ?? []).filter((entry) => entry.verdict === 'close'), [entries]);
  const kept = useMemo(() => (entries ?? []).filter((entry) => entry.verdict === 'keep'), [entries]);
  const closeMb = closeCandidates.reduce((sum, entry) => sum + entry.memoryKb, 0) / 1024;

  const run = async () => {
    setBusy(true);
    setError('');
    try {
      const report = await invoke<PlayGameReport>('optimize_and_launch_game', {
        appId: Number(game.appId),
        minMemoryMb: RAM_DEFAULT_MIN_MB,
      });
      setResult(report);
    } catch (err: unknown) {
      setError(toErrorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const report: RamOptimizeReport | null = result?.report ?? null;
  const notInstalled = game.installed === false || game.installing === true;

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
          {report && (
            <>
              <p className="uninstall-modal-copy">
                Closed {report.closed.length} process(es) and reclaimed about {formatMb(report.reclaimedKb)}.
              </p>
              {report.closed.length > 0 && (
                <ul className="play-process-list">
                  {report.closed.map((name) => (
                    <li key={name}>{name}</li>
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
            Steam, AetherDesk, anti-cheat, overlays, other launchers and every running game are never closed. The list
            below is a preview — nothing is closed until you confirm.
          </p>

          {error && <p className="uninstall-modal-copy play-modal-error">{error}</p>}

          {entries === null && !error ? (
            <p className="uninstall-modal-copy">Reading the process list…</p>
          ) : (
            <>
              <p className="uninstall-modal-copy">
                Close candidates: <strong>{closeCandidates.length}</strong> ({formatMb(closeMb)}) &nbsp;·&nbsp;kept:{' '}
                <strong>{kept.length}</strong>
              </p>
              <ul className="play-process-list">
                {closeCandidates.slice(0, 40).map((entry) => (
                  <li key={entry.pid}>
                    <span className="play-process-name">{entry.name}</span>
                    <span className="play-process-meta">
                      {formatMb(entry.memoryKb)} · PID {entry.pid}
                    </span>
                  </li>
                ))}
                {closeCandidates.length > 40 && (
                  <li className="play-process-more">…and {closeCandidates.length - 40} more</li>
                )}
              </ul>
            </>
          )}

          <div className="uninstall-modal-actions">
            <button
              type="button"
              className="uninstall-btn uninstall-btn-danger"
              disabled={busy || closeCandidates.length === 0}
              onClick={run}
            >
              {busy ? 'Optimizing…' : closeCandidates.length === 0 ? 'No candidate to close' : 'Optimize & Play'}
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
