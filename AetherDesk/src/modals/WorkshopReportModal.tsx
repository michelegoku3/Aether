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
  switch (item.reason) {
    case 'hubcapNotFound':
      return "Probably not in Hubcap's database.";
    case 'unknownError':
      return 'Unknown error.';
    case 'pendingSubscription':
      return 'Steam has the subscription but has not installed it yet.';
    case 'generationCap':
      return 'Deferred by the generation limit; retried on the next pass.';
    case 'downloadCap':
      return 'Manifest is ready; the download request was deferred to the next pass.';
    default:
      return null;
  }
};

const badgeText = (item: WorkshopItemReport): string => {
  if (item.status === 'pending') return 'subscription pending';
  if (item.status === 'deferred') {
    return item.reason === 'generationCap' ? 'generation cap' : 'download cap';
  }
  if (item.status === 'unavailable' || item.reason === 'hubcapNotFound') return '404';
  if (item.status === 'failed') return 'error';
  return 'ready';
};

const ReportRow = ({ item }: { item: WorkshopItemReport }) => {
  const reason = reasonLabel(item);
  const extras = [item.contentEmpty ? 'empty content folder' : null, item.needsDownload ? 'Steam is retrying' : null]
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
 * It opens instead of the repair dialog whenever a run has failures, pending
 * subscriptions or deferred items. The list is the actionable part: what can
 * download now, what waits for the next pass, and what Hubcap did not answer
 * for. The footer note asks the user to unsubscribe from the failed mods; no
 * action edits Steam's ACF or subscription state.
 */
export const WorkshopReportModal = ({ report, error, onClose }: WorkshopReportModalProps) => {
  const ready = report?.items.filter((item) => item.status === 'ready' || item.status === 'pending') ?? [];
  const deferred = report?.items.filter((item) => item.status === 'deferred') ?? [];
  const errors = report?.items.filter((item) => item.status === 'failed' || item.status === 'unavailable') ?? [];
  const errorLabel = errors.length === 1 ? 'error' : 'errors';
  const summary = report
    ? `${report.discovered} installed · ${report.alreadyLocal} already local · ${ready.length} to download · ${deferred.length} deferred · ${errors.length} ${errorLabel}`
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
