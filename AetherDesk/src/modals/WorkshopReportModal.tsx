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

type ReportGroup = 'ready' | 'deferred' | 'errors';

const gameName = (item: WorkshopItemReport): string => item.appName ?? `App ${item.appId}`;

const modTitle = (item: WorkshopItemReport): string => item.title?.trim() || `Mod #${item.workshopId}`;

const reasonLabel = (item: WorkshopItemReport): string | null => {
  if (item.status === 'ready') {
    return item.update
      ? 'New manifest staged; start the download in Steam.'
      : 'Manifest staged; start the download in Steam.';
  }
  switch (item.reason) {
    case 'hubcapNotFound':
      return "Probably not in Hubcap's database.";
    case 'unknownError':
      return 'Unknown error.';
    case 'generationCap':
      return 'Deferred by the per-run limit; run Repair again to retry.';
    default:
      return null;
  }
};

const badgeText = (item: WorkshopItemReport): string => {
  if (item.status === 'deferred') return 'generation cap';
  if (item.status === 'unavailable' || item.reason === 'hubcapNotFound') return '404';
  if (item.status === 'failed') return 'error';
  return item.update ? 'update' : 'ready';
};

const compareText = (left: string, right: string): number =>
  left.localeCompare(right, undefined, { sensitivity: 'base' });

/**
 * The order inside a section is the grouping the report promises: error type
 * first, then game, then mod title. Games stay contiguous because the title is
 * only compared after the game.
 */
const reasonRank = (item: WorkshopItemReport, group: ReportGroup): number => {
  if (group === 'errors') {
    if (item.reason === 'hubcapNotFound') return 0;
    if (item.reason === 'unknownError') return 1;
    return 2;
  }
  if (group === 'deferred') {
    if (item.reason === 'generationCap') return 0;
    return 2;
  }
  return 0;
};

const sortItems = (items: WorkshopItemReport[], group: ReportGroup): WorkshopItemReport[] =>
  [...items].sort((left, right) => {
    const rank = reasonRank(left, group) - reasonRank(right, group);
    if (rank !== 0) return rank;
    const game = compareText(gameName(left), gameName(right));
    if (game !== 0) return game;
    return compareText(modTitle(left), modTitle(right));
  });

const ReportRow = ({ item }: { item: WorkshopItemReport }) => {
  const reason = reasonLabel(item);

  return (
    <div className="workshop-report-row">
      <div className="workshop-report-row-main">
        <div className="workshop-report-row-name" title={modTitle(item)}>
          {modTitle(item)}
        </div>
        <div className="workshop-report-row-meta">
          <span>{`Game: ${gameName(item)} - Mod ID: #${item.workshopId} - Manifest: `}</span>
          <span className="workshop-report-row-manifest">{item.manifestGid}</span>
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
  const ready = sortItems(report?.items.filter((item) => item.status === 'ready') ?? [], 'ready');
  const deferred = sortItems(report?.items.filter((item) => item.status === 'deferred') ?? [], 'deferred');
  const errors = sortItems(
    report?.items.filter((item) => item.status === 'failed' || item.status === 'unavailable') ?? [],
    'errors'
  );
  const errorLabel = errors.length === 1 ? 'error' : 'errors';
  const summary = report
    ? `${report.discovered} installed - ${report.alreadyLocal} already local - ${ready.length} ready - ${deferred.length} deferred - ${errors.length} ${errorLabel}`
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
