import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { WorkshopReportModal } from './WorkshopReportModal';
import type { WorkshopSyncReport } from './WorkshopRepairModal';

const REPORT: WorkshopSyncReport = {
  discovered: 5,
  generated: 0,
  restoredFromCache: 0,
  alreadyLocal: 2,
  contentMissing: 2,
  failed: 1,
  deferred: 1,
  pending: 1,
  unavailable: 0,
  items: [
    {
      appId: 3328490,
      appName: 'Airport Control 27',
      workshopId: 3806068832,
      manifestGid: '3748992252827399921',
      status: 'ready',
      reason: null,
      httpStatus: null,
      contentEmpty: false,
      needsDownload: false,
    },
    {
      appId: 431960,
      appName: 'Wallpaper Engine',
      workshopId: 111,
      manifestGid: '2',
      status: 'pending',
      reason: 'pendingSubscription',
      httpStatus: null,
      contentEmpty: false,
      needsDownload: true,
    },
    {
      appId: 3328490,
      appName: 'Airport Control 27',
      workshopId: 222,
      manifestGid: '3',
      status: 'deferred',
      reason: 'downloadCap',
      httpStatus: null,
      contentEmpty: true,
      needsDownload: false,
    },
    {
      appId: 3328490,
      appName: 'Airport Control 27',
      workshopId: 333,
      manifestGid: '4',
      status: 'failed',
      reason: 'hubcapNotFound',
      httpStatus: 404,
      contentEmpty: false,
      needsDownload: false,
    },
  ],
};

describe('WorkshopReportModal', () => {
  it('shows ready, deferred and error sections', () => {
    render(<WorkshopReportModal report={REPORT} error={null} onClose={vi.fn()} />);

    expect(screen.getByText('Ready to download')).toBeInTheDocument();
    expect(screen.getByText('Deferred')).toBeInTheDocument();
    expect(screen.getByText('Errors')).toBeInTheDocument();
    expect(screen.getAllByText('Airport Control 27').length).toBeGreaterThan(0);
    expect(screen.getByText('#3806068832')).toBeInTheDocument();
    expect(screen.getByText('3748992252827399921')).toBeInTheDocument();
    expect(screen.getByText(/1 error$/)).toBeInTheDocument();
    expect(screen.getByText('404')).toBeInTheDocument();
    expect(screen.getByText(/Probably not in Hubcap's database/)).toBeInTheDocument();
  });

  it('marks pending subscriptions', () => {
    render(<WorkshopReportModal report={REPORT} error={null} onClose={vi.fn()} />);

    expect(screen.getByText('Wallpaper Engine')).toBeInTheDocument();
    expect(screen.getByText('subscription pending')).toBeInTheDocument();
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
