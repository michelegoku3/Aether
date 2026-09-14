import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ModalShell } from '../ui/ModalShell';
import { StatusType } from '../types/ui';

/** Report returned by the backend `sync_hubcap_workshop_manifests` command. */
export interface WorkshopSyncReport {
  discovered: number;
  generated: number;
  restoredFromCache: number;
  alreadyLocal: number;
  /** Items with a staged manifest but without Steam's content folder. */
  contentMissing: number;
  failed: number;
  /** Items beyond the per-run generation cap; the next lane pass handles them. */
  deferred: number;
}

export interface WorkshopRepairModalProps {
  /** Toast feedback in the hosting view (same StatusAlert as other actions). */
  onStatus: (text: string, type: StatusType) => void;
  /** X / Escape / overlay — dismiss (disabled while the sync runs). */
  onClose: () => void;
}

/** Human-readable one-liner for the sync report. */
const formatReport = (report: WorkshopSyncReport): string => {
  if (report.discovered === 0) {
    return 'No installed Steam Workshop items were found.';
  }
  const parts = [
    `${report.discovered} item(s) discovered`,
    `${report.generated} generated`,
    `${report.restoredFromCache} restored from cache`,
    `${report.alreadyLocal} already local`,
  ];
  if (report.contentMissing > 0) {
    parts.push(`${report.contentMissing} missing content (download requested from Steam)`);
  }
  if (report.failed > 0) {
    parts.push(`${report.failed} failed`);
  }
  if (report.deferred > 0) {
    parts.push(`${report.deferred} deferred to the next pass (generation cap)`);
  }
  return `Workshop repair: ${parts.join(', ')}.`;
};

/**
 * Explicit, user-triggered Steam Workshop manifest repair.
 *
 * The backend scans Steam's `appworkshop_*.acf` files and stages every
 * missing item manifest — exact local files and the validated local cache
 * are reused first, everything else is generated through the authenticated
 * Hubcap key (Workshop quota). Items whose content folder is absent get
 * their download requested from the running Steam client. Completed items
 * never touch the provider.
 */
export const WorkshopRepairModal = ({ onStatus, onClose }: WorkshopRepairModalProps) => {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);

  const runRepair = async () => {
    if (busy) return;
    setBusy(true);
    setResult(null);
    onStatus('Repairing Steam Workshop manifests...', 'info');
    try {
      const report = await invoke<WorkshopSyncReport>('sync_hubcap_workshop_manifests');
      const summary = formatReport(report);
      setResult(summary);
      onStatus(summary, report.failed > 0 ? 'error' : 'success');
    } catch (err: any) {
      const message = `Workshop repair failed: ${err}`;
      setResult(message);
      onStatus(message, 'error');
    } finally {
      setBusy(false);
    }
  };

  return (
    <ModalShell
      title="Repair Workshop"
      onClose={() => {
        if (!busy) onClose();
      }}
      closeDisabled={busy}
      containerClassName="uninstall-modal"
      bodyClassName="uninstall-modal-body"
    >
      <p className="uninstall-modal-copy">
        Scan every installed Steam Workshop item and stage its manifest: files already in
        depotcache or in the local cache are reused, everything else is generated through
        your authenticated Hubcap key. Mods whose installation folder is missing are
        requested from Steam automatically.
      </p>
      {busy && <p className="uninstall-modal-copy">Repairing Workshop manifests…</p>}
      {!busy && result && <p className="uninstall-modal-copy">{result}</p>}

      <div className="uninstall-modal-actions">
        <button
          type="button"
          className="uninstall-btn uninstall-btn-primary"
          onClick={() => void runRepair()}
          disabled={busy}
        >
          {busy ? 'Repairing…' : 'Repair Workshop manifests'}
        </button>
      </div>
    </ModalShell>
  );
};
