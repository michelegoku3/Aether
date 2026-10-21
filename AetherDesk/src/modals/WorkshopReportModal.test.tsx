import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { WorkshopReportModal } from './WorkshopReportModal';
import type { WorkshopSyncReport } from './WorkshopRepairModal';

const item = (overrides: Partial<WorkshopSyncReport['items'][number]> = {}): WorkshopSyncReport['items'][number] => ({
  appId: 3328490,
  appName: 'Airport Control 27',
  workshopId: 3806068832,
  title: 'Airport Control 27',
  manifestGid: '3748992252827399921',
  status: 'ready',
  reason: null,
  httpStatus: null,
  contentEmpty: false,
  needsDownload: false,
  subscription: false,
  update: false,
  ...overrides,
});

const REPORT: WorkshopSyncReport = {
  discovered: 5,
  subscriptions: 1,
  generated: 1,
  restoredFromCache: 0,
  alreadyLocal: 2,
  contentMissing: 1,
  updates: 1,
  failed: 1,
  deferred: 1,
  unavailable: 0,
  items: [
    item(),
    item({
      appId: 431960,
      appName: 'Wallpaper Engine',
      workshopId: 111,
      title: 'Wallpaper Engine',
      manifestGid: '2',
      subscription: true,
      update: true,
    }),
    item({
      workshopId: 222,
      title: 'Airport Control 27 - Deferred',
      manifestGid: '3',
      status: 'deferred',
      reason: 'generationCap',
      contentEmpty: true,
    }),
    item({
      workshopId: 333,
      title: 'Airport Control 27 - Broken',
      manifestGid: '4',
      status: 'failed',
      reason: 'hubcapNotFound',
      httpStatus: 404,
    }),
  ],
};

const rowNames = (): Array<string | null> =>
  Array.from(document.querySelectorAll('.workshop-report-row-name')).map((node) => node.textContent);

describe('WorkshopReportModal', () => {
  it('shows ready, deferred and error sections with a compact summary', () => {
    render(<WorkshopReportModal report={REPORT} error={null} onClose={vi.fn()} />);

    expect(screen.getByText('Ready to download')).toBeInTheDocument();
    expect(screen.getByText('Deferred')).toBeInTheDocument();
    expect(screen.getByText('Errors')).toBeInTheDocument();
    expect(screen.getByText('5 installed - 2 already local - 2 ready - 1 deferred - 1 error')).toBeInTheDocument();
    expect(screen.getByText('Airport Control 27')).toBeInTheDocument();
    expect(screen.getByText(/Game: Airport Control 27 - Mod ID: #3806068832 - Manifest:/)).toBeInTheDocument();
    expect(screen.getByText('3748992252827399921')).toBeInTheDocument();
    expect(screen.getByText('404')).toBeInTheDocument();
    expect(screen.getByText(/Probably not in Hubcap's database/)).toBeInTheDocument();
    expect(screen.queryByText(/subscription/)).not.toBeInTheDocument();
  });

  it('shows update and empty-content extras without subscription or retry wording', () => {
    render(<WorkshopReportModal report={REPORT} error={null} onClose={vi.fn()} />);

    expect(screen.getByText('Wallpaper Engine')).toBeInTheDocument();
    expect(screen.getAllByText('update').length).toBeGreaterThan(0);
    expect(screen.getByText('empty content folder')).toBeInTheDocument();
    expect(screen.getByText(/New manifest staged; start the download in Steam/)).toBeInTheDocument();
    expect(screen.queryByText(/Steam is retrying/)).not.toBeInTheDocument();
    expect(screen.queryByText(/subscription/)).not.toBeInTheDocument();
  });

  it('groups errors by error type, then game, then mod title', () => {
    const report: WorkshopSyncReport = {
      ...REPORT,
      failed: 4,
      items: [
        item({
          appId: 1,
          appName: 'Zeta Game',
          workshopId: 1,
          title: 'Alpha Mod',
          status: 'failed',
          reason: 'unknownError',
          httpStatus: 500,
        }),
        item({
          appId: 2,
          appName: 'Alpha Game',
          workshopId: 2,
          title: 'Beta Mod',
          status: 'unavailable',
          reason: 'hubcapNotFound',
          httpStatus: 404,
        }),
        item({
          appId: 2,
          appName: 'Alpha Game',
          workshopId: 3,
          title: 'Alpha Mod',
          status: 'failed',
          reason: 'hubcapNotFound',
          httpStatus: 404,
        }),
        item({
          appId: 1,
          appName: 'Zeta Game',
          workshopId: 4,
          title: 'Zeta Mod',
          status: 'failed',
          reason: 'hubcapNotFound',
          httpStatus: 404,
        }),
      ],
    };

    render(<WorkshopReportModal report={report} error={null} onClose={vi.fn()} />);

    expect(rowNames()).toEqual(['Alpha Mod', 'Beta Mod', 'Zeta Mod', 'Alpha Mod']);
  });

  it('groups deferred items by game, then mod title', () => {
    const report: WorkshopSyncReport = {
      ...REPORT,
      deferred: 3,
      items: [
        item({
          appId: 1,
          appName: 'Zeta Game',
          workshopId: 2,
          title: 'Beta Mod',
          status: 'deferred',
          reason: 'generationCap',
        }),
        item({
          appId: 2,
          appName: 'Alpha Game',
          workshopId: 3,
          title: 'Zeta Mod',
          status: 'deferred',
          reason: 'generationCap',
        }),
        item({
          appId: 2,
          appName: 'Alpha Game',
          workshopId: 4,
          title: 'Alpha Mod',
          status: 'deferred',
          reason: 'generationCap',
        }),
      ],
    };

    render(<WorkshopReportModal report={report} error={null} onClose={vi.fn()} />);

    expect(rowNames()).toEqual(['Alpha Mod', 'Zeta Mod', 'Beta Mod']);
  });

  it('sorts ready items by game and mod title and falls back to the mod id', () => {
    const report: WorkshopSyncReport = {
      ...REPORT,
      items: [
        item({ appId: 1, appName: 'Zeta Game', workshopId: 1, title: 'Alpha Mod' }),
        item({ appId: 2, appName: 'Alpha Game', workshopId: 2, title: 'Zeta Mod' }),
        item({ appId: 2, appName: 'Alpha Game', workshopId: 3, title: 'Alpha Mod' }),
        item({
          appId: 3,
          appName: 'Fallback Game',
          workshopId: 3806068832,
          title: null,
          status: 'failed',
          reason: 'unknownError',
        }),
      ],
    };

    render(<WorkshopReportModal report={report} error={null} onClose={vi.fn()} />);

    expect(rowNames()).toEqual(['Alpha Mod', 'Zeta Mod', 'Alpha Mod', 'Mod #3806068832']);
  });

  it('shows the unsubscribe note', () => {
    render(<WorkshopReportModal report={REPORT} error={null} onClose={vi.fn()} />);

    expect(screen.getByText(/Unsubscribe from them on the Steam Workshop page/)).toBeInTheDocument();
  });

  it('shows only the command error when there is no report', () => {
    render(
      <WorkshopReportModal
        report={null}
        error="Workshop repair failed: Hubcap API key is not valid."
        onClose={vi.fn()}
      />
    );

    expect(screen.getByText(/Hubcap API key is not valid/)).toBeInTheDocument();
    expect(screen.queryByText('Ready to download')).not.toBeInTheDocument();
  });

  it('calls onClose', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<WorkshopReportModal report={REPORT} error={null} onClose={onClose} />);

    await user.click(screen.getByRole('button', { name: 'Close report' }));
    expect(onClose).toHaveBeenCalledOnce();
  });
});
