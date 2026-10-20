import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ModalShell } from '../ui/ModalShell';
import { StatusType } from '../types/ui';
import { toErrorMessage } from '../util/errors';

export type WorkshopItemStatus = 'ready' | 'deferred' | 'failed' | 'unavailable';

export type WorkshopItemReason = 'hubcapNotFound' | 'unknownError' | 'generationCap' | 'downloadCap';

export interface WorkshopItemReport {
  appId: number;
  appName: string | null;
  workshopId: number;
  /** Steam manifest GIDs exceed JavaScript's safe integer; serialize as text. */
  manifestGid: string;
  status: WorkshopItemStatus;
  /** Null for a ready item: being ready is not a failure. */
  reason: WorkshopItemReason | null;
  httpStatus: number | null;
  /** The content directory existed but had no entries. */
  contentEmpty: boolean;
  /** Steam's ACF says it has a pending Workshop download. */
  needsDownload: boolean;
  /** The item came only from `WorkshopItemDetails`. */
  subscription: boolean;
  /** The target manifest is newer than the installed one. */
  update: boolean;
  /** Aether asked Steam to download the payload in this run. */
  downloadRequested: boolean;
}

/** Report returned by the backend `sync_hubcap_workshop_manifests` command. */
export interface WorkshopSyncReport {
  /** Installed Workshop items considered in this run. */
  discovered: number;
  /** `WorkshopItemDetails` subscriptions considered in this run. */
  subscriptions: number;
  generated: number;
  restoredFromCache: number;
  alreadyLocal: number;
  /** Items with a staged manifest but without Steam's content folder. */
  contentMissing: number;
  /** Installed items whose target manifest is newer than the installed one. */
  updates: number;
  /** Items for which Aether asked Steam to download the payload. */
  downloadRequested: number;
  failed: number;
  /** Items beyond the per-run generation or download cap. */
  deferred: number;
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
  if (report.discovered === 0 && report.subscriptions === 0) {
    return 'No installed Steam Workshop items or subscriptions were found.';
  }
  const parts = [
    `${report.discovered} installed item(s)`,
    `${report.generated} generated`,
    `${report.restoredFromCache} restored from cache`,
    `${report.alreadyLocal} already local`,
  ];
  if (report.subscriptions > 0) {
    parts.push(`${report.subscriptions} subscription(s)`);
  }
  if (report.updates > 0) {
    parts.push(`${report.updates} update(s)`);
  }
  if (report.contentMissing > 0) {
    parts.push(`${report.contentMissing} ready for download`);
  }
  if (report.downloadRequested > 0) {
    parts.push(`${report.downloadRequested} download(s) requested`);
  } else if (report.updates > 0 || report.contentMissing > 0) {
    parts.push('start the download in Steam');
  }
  return `Workshop repair complete: ${parts.join(', ')}.`;
};

/** A result that needs the user's attention, not a success toast. */
const needsReport = (report: WorkshopSyncReport): boolean =>
  report.failed > 0 || report.deferred > 0 || report.unavailable > 0;

/**
 * Explicit, user-triggered Steam Workshop manifest repair.
 *
 * The backend scans Steam's `appworkshop_*.acf` files and stages every
 * missing item manifest — exact local files and the validated local cache
 * are reused first, everything else is generated through the authenticated
 * Hubcap key (Workshop quota). The manual action also targets
 * `WorkshopItemDetails` subscriptions; the background monitor does not. A clean
 * run closes this popup with a toast; anything that needs attention opens the
 * dedicated report popup instead.
 */
export const WorkshopRepairModal = ({ onStatus, onReport, onClose }: WorkshopRepairModalProps) => {
  const [busy, setBusy] = useState(false);

  const runRepair = async () => {
    if (busy) return;
    setBusy(true);
    let report: WorkshopSyncReport;
    try {
      report = await invoke<WorkshopSyncReport>('sync_hubcap_workshop_manifests', {
        includeSubscriptions: true,
      });
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
    if (report.discovered === 0 && report.subscriptions === 0) {
      onStatus('No installed Steam Workshop items or subscriptions were found.', 'info');
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
        Scan installed Steam Workshop items and subscriptions Steam has not installed yet, then stage their manifests:
        files already in depotcache or in the local cache are reused, everything else is generated through your
        authenticated Hubcap key. Missing content and updates are requested from Steam automatically.
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
