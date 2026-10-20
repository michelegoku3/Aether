import { ModalShell } from '../ui/ModalShell';
import { StatusAlert } from '../ui/StatusAlert';
import type { WorkshopItemReport, WorkshopSyncReport } from './WorkshopRepairModal';

export interface WorkshopReportModalProps {
  /** The sync result; null when the command itself failed. */
  report: WorkshopSyncReport | null;
  /** Command-level error text; takes precedence over `report`. */
  error: string | null;
  onClose: () => void;
}

const itemName = (item: WorkshopItemReport): string => item.appName ?? `App ${item.appId}`;

const reasonLabel = (item: WorkshopItemReport): string | null => {
  if (item.status === 'ready' && item.update) {
    return item.downloadRequested
      ? 'New manifest staged; Steam download requested.'
      : 'New manifest staged; start the download in Steam.';
  }
  switch (item.reason) {
    case 'hubcapNotFound':
      return "Probably not in Hubcap's database.";
    case 'unknownError':
      return 'Unknown error.';
    case 'generationCap':
      return 'Deferred by the per-run limit; run Repair again to retry.';
    case 'downloadCap':
      return 'Manifest is ready; run Repair again to request the remaining Steam downloads.';
    default:
      return null;
  }
};

const badgeText = (item: WorkshopItemReport): string => {
  if (item.status === 'deferred') {
    return item.reason === 'generationCap' ? 'generation cap' : 'download cap';
  }
  if (item.status === 'unavailable' || item.reason === 'hubcapNotFound') return '404';
  if (item.status === 'failed') return 'error';
  return item.update ? 'update' : 'ready';
};

const ReportRow = ({ item }: { item: WorkshopItemReport }) => {
  const reason = reasonLabel(item);
  const extras = [
    item.subscription ? 'subscription' : null,
    item.update ? 'update' : null,
    item.contentEmpty ? 'empty content folder' : null,
    item.needsDownload ? 'Steam is retrying' : null,
  ]
    .filter(Boolean)
    .join(' · ');

  return (
    <div className="workshop-report-row">
      <div className="workshop-report-row-main">
        <div className="workshop-report-row-name">{itemName(item)}</div>
        <div className="workshop-report-row-meta">
          <span>#{item.workshopId}</span>
          <span> · manifest </span>
          <span className="workshop-report-row-manifest">{item.manifestGid}</span>
          {extras ? <span>{` · ${extras}`}</span> : null}
        </div>
        {reason && <div className="workshop-report-row-reason">{reason}</div>}
      </div>
      <span
        className={`workshop-report-badge${item.status === 'failed' || item.status === 'unavailable' ? ' error' : ''}`}
      >
        {badgeText(item)}
      </span>
    </div>
  );
};

const ReportSection = ({ title, items }: { title: string; items: WorkshopItemReport[] }) => {
  if (items.length === 0) return null;
  return (
    <section className="workshop-report-section">
      <h4 className="workshop-report-section-title">{title}</h4>
      {items.map((item) => (
        <ReportRow key={`${item.appId}-${item.workshopId}`} item={item} />
      ))}
    </section>
  );
};

/**
 * Dedicated result popup for Workshop repair.
 *
 * It opens instead of the repair dialog whenever a run has failures or deferred
 * items. The list is the actionable part: what can download now, what waits for
 * the next pass, and what Hubcap did not answer for. The footer note asks the
 * user to unsubscribe from the failed mods; no action edits Steam's ACF or
 * subscription state.
 */
export const WorkshopReportModal = ({ report, error, onClose }: WorkshopReportModalProps) => {
  const ready = report?.items.filter((item) => item.status === 'ready') ?? [];
  const deferred = report?.items.filter((item) => item.status === 'deferred') ?? [];
  const errors = report?.items.filter((item) => item.status === 'failed' || item.status === 'unavailable') ?? [];
  const errorLabel = errors.length === 1 ? 'error' : 'errors';
  const subscriptionLabel = (report?.subscriptions ?? 0) === 1 ? 'subscription' : 'subscriptions';
  const summary = report
    ? `${report.discovered} installed · ${report.subscriptions} ${subscriptionLabel} · ${report.alreadyLocal} already local · ${ready.length} ready · ${deferred.length} deferred · ${errors.length} ${errorLabel}`
    : null;

  return (
    <ModalShell
      title="Workshop report"
      onClose={onClose}
      containerClassName="workshop-report-modal"
      bodyClassName="workshop-report-body"
    >
      {error ? (
        <StatusAlert status={{ text: error, type: 'error' }} className="settings-alert--compact" />
      ) : report ? (
        <>
          <p className="workshop-report-summary">{summary}</p>
          <div className="workshop-report-list">
            <ReportSection title="Ready to download" items={ready} />
            <ReportSection title="Deferred" items={deferred} />
            <ReportSection title="Errors" items={errors} />
          </div>
          {errors.length > 0 && (
            <p className="workshop-report-note">
              The mods with errors are probably not in Hubcap&apos;s database yet, or an unknown error occurred.
              Unsubscribe from them on the Steam Workshop page to stop Steam from retrying their downloads.
            </p>
          )}
        </>
      ) : (
        <p className="uninstall-modal-copy">No report available.</p>
      )}

      <div className="uninstall-modal-actions">
        <button
          type="button"
          className="uninstall-btn uninstall-btn-primary"
          aria-label="Close report"
          onClick={onClose}
        >
          Close
        </button>
      </div>
    </ModalShell>
  );
};
