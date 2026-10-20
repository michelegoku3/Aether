import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ModalShell } from '../ui/ModalShell';
import { StatusType } from '../types/ui';
import { toErrorMessage } from '../util/errors';

export type WorkshopItemStatus = 'ready' | 'pending' | 'deferred' | 'failed' | 'unavailable';

export type WorkshopItemReason =
  | 'hubcapNotFound'
  | 'unknownError'
  | 'pendingSubscription'
  | 'generationCap'
  | 'downloadCap';

export interface WorkshopItemReport {
  appId: number;
  appName: string | null;
  workshopId: number;
  manifestGid: number;
  status: WorkshopItemStatus;
  /** Null for a ready item: being ready is not a failure. */
  reason: WorkshopItemReason | null;
  httpStatus: number | null;
  /** The content directory existed but had no entries. */
  contentEmpty: boolean;
  /** Steam's ACF says it has a pending Workshop download. */
  needsDownload: boolean;
}

/** Report returned by the backend `sync_hubcap_workshop_manifests` command. */
export interface WorkshopSyncReport {
  discovered: number;
  generated: number;
  restoredFromCache: number;
  alreadyLocal: number;
  /** Items with a staged manifest but without Steam's content folder. */
  contentMissing: number;
  failed: number;
  /** Items beyond the per-run generation or download cap. */
  deferred: number;
  /** Subscriptions in `WorkshopItemDetails` that Steam has not installed. */
  pending: number;
  /** Items skipped by the 24 h terminal-404 cache. */
  unavailable: number;
  items: WorkshopItemReport[];
}

export interface WorkshopRepairModalProps {
  /** Toast feedback in the hosting view (same StatusAlert as other actions). */
  onStatus: (text: string, type: StatusType) => void;
  /**
   * Hand the result to the dedicated report popup. The parent closes this
   * popup before mounting the report, so a non-success result never hides
   * behind the repair dialog.
   */
  onReport: (report: WorkshopSyncReport | null, error: string | null) => void;
  /** X / Escape / overlay — dismiss (disabled while the sync runs). */
  onClose: () => void;
}

/** Human-readable one-liner for the success toast. */
const formatReport = (report: WorkshopSyncReport): string => {
  if (report.discovered === 0 && report.pending === 0) {
    return 'No installed Steam Workshop items were found.';
  }
  const parts = [
    `${report.discovered} installed item(s)`,
    `${report.generated} generated`,
    `${report.restoredFromCache} restored from cache`,
    `${report.alreadyLocal} already local`,
  ];
  if (report.contentMissing > 0) {
    parts.push(`${report.contentMissing} ready for download`);
  }
  if (report.pending > 0) {
    parts.push(`${report.pending} subscription(s) pending`);
  }
  if (report.deferred > 0) {
    parts.push(`${report.deferred} deferred`);
  }
  if (report.failed > 0) {
    parts.push(`${report.failed} failed`);
  }
  if (report.unavailable > 0) {
    parts.push(`${report.unavailable} unavailable`);
  }
  return `Workshop repair complete: ${parts.join(', ')}.`;
};

/** A result that needs the user's attention, not a success toast. */
const needsReport = (report: WorkshopSyncReport): boolean =>
  report.failed > 0 || report.pending > 0 || report.deferred > 0 || report.unavailable > 0;

/**
 * Explicit, user-triggered Steam Workshop manifest repair.
 *
 * The backend scans Steam's `appworkshop_*.acf` files and stages every
 * missing item manifest — exact local files and the validated local cache
 * are reused first, everything else is generated through the authenticated
 * Hubcap key (Workshop quota). Pending subscriptions are only asked to Steam,
 * never sent to Hubcap. A clean run closes this popup with a toast; anything
 * that needs attention opens the dedicated report popup instead.
 */
export const WorkshopRepairModal = ({ onStatus, onReport, onClose }: WorkshopRepairModalProps) => {
  const [busy, setBusy] = useState(false);

  const runRepair = async () => {
    if (busy) return;
    setBusy(true);
    let report: WorkshopSyncReport;
    try {
      report = await invoke<WorkshopSyncReport>('sync_hubcap_workshop_manifests');
    } catch (err: unknown) {
      setBusy(false);
      onReport(null, `Workshop repair failed: ${toErrorMessage(err)}`);
      return;
    }
    setBusy(false);

    if (needsReport(report)) {
      onReport(report, null);
      return;
    }
    if (report.discovered === 0 && report.pending === 0) {
      onStatus('No installed Steam Workshop items were found.', 'info');
      onClose();
      return;
    }
    onStatus(formatReport(report), 'success');
    onClose();
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
        Scan every installed Steam Workshop item and stage its manifest: files already in depotcache or in the local
        cache are reused, everything else is generated through your authenticated Hubcap key. Mods whose installation
        folder is missing or empty are requested from Steam automatically.
      </p>
      {busy && <p className="uninstall-modal-copy">Repairing Workshop manifests…</p>}

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
