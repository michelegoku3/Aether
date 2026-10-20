import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { invokeMock, mockInvokeCommands } from '../test/tauriMocks';
import { WorkshopRepairModal, type WorkshopSyncReport } from './WorkshopRepairModal';

const report = (overrides: Partial<WorkshopSyncReport> = {}): WorkshopSyncReport => ({
  discovered: 4,
  subscriptions: 0,
  generated: 0,
  restoredFromCache: 0,
  alreadyLocal: 4,
  contentMissing: 0,
  updates: 0,
  downloadRequested: 0,
  failed: 0,
  deferred: 0,
  unavailable: 0,
  items: [],
  ...overrides,
});

const reportItem = (
  overrides: Partial<WorkshopSyncReport['items'][number]> = {}
): WorkshopSyncReport['items'][number] => ({
  appId: 3328490,
  appName: 'Airport Control 27',
  workshopId: 3806068832,
  manifestGid: '1',
  status: 'failed',
  reason: 'hubcapNotFound',
  httpStatus: 404,
  contentEmpty: false,
  needsDownload: false,
  subscription: false,
  update: false,
  downloadRequested: false,
  ...overrides,
});

describe('WorkshopRepairModal', () => {
  it('closes with a success toast when nothing needs attention', async () => {
    const user = userEvent.setup();
    const onStatus = vi.fn();
    const onReport = vi.fn();
    const onClose = vi.fn();
    mockInvokeCommands({ sync_hubcap_workshop_manifests: report() });

    render(<WorkshopRepairModal onStatus={onStatus} onReport={onReport} onClose={onClose} />);
    await user.click(screen.getByRole('button', { name: 'Repair Workshop manifests' }));

    await waitFor(() => expect(onClose).toHaveBeenCalledOnce());
    expect(onStatus).toHaveBeenCalledWith(expect.stringContaining('4 already local'), 'success');
    expect(onReport).not.toHaveBeenCalled();
    expect(invokeMock).toHaveBeenCalledWith('sync_hubcap_workshop_manifests', {
      includeSubscriptions: true,
    });
  });

  it('reports a repaired subscription and update in the success toast', async () => {
    const user = userEvent.setup();
    const onStatus = vi.fn();
    const onReport = vi.fn();
    const onClose = vi.fn();
    mockInvokeCommands({
      sync_hubcap_workshop_manifests: report({
        discovered: 4,
        subscriptions: 1,
        generated: 2,
        contentMissing: 1,
        updates: 1,
        downloadRequested: 2,
      }),
    });

    render(<WorkshopRepairModal onStatus={onStatus} onReport={onReport} onClose={onClose} />);
    await user.click(screen.getByRole('button', { name: 'Repair Workshop manifests' }));

    await waitFor(() => expect(onClose).toHaveBeenCalledOnce());
    const message = onStatus.mock.calls[0][0] as string;
    expect(message).toContain('1 subscription');
    expect(message).toContain('1 update');
    expect(message).toContain('2 download(s) requested');
    expect(onReport).not.toHaveBeenCalled();
  });

  it('hands failures to the dedicated report popup', async () => {
    const user = userEvent.setup();
    const onStatus = vi.fn();
    const onReport = vi.fn();
    const onClose = vi.fn();
    const failed = report({
      subscriptions: 1,
      failed: 1,
      items: [reportItem()],
    });
    mockInvokeCommands({ sync_hubcap_workshop_manifests: failed });

    render(<WorkshopRepairModal onStatus={onStatus} onReport={onReport} onClose={onClose} />);
    await user.click(screen.getByRole('button', { name: 'Repair Workshop manifests' }));

    await waitFor(() => expect(onReport).toHaveBeenCalledWith(failed, null));
    expect(onStatus).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('hands a command error to the report popup instead of a toast', async () => {
    const user = userEvent.setup();
    const onStatus = vi.fn();
    const onReport = vi.fn();
    const onClose = vi.fn();
    mockInvokeCommands({
      sync_hubcap_workshop_manifests: () => {
        throw new Error('Hubcap API key is not valid.');
      },
    });

    render(<WorkshopRepairModal onStatus={onStatus} onReport={onReport} onClose={onClose} />);
    await user.click(screen.getByRole('button', { name: 'Repair Workshop manifests' }));

    await waitFor(() =>
      expect(onReport).toHaveBeenCalledWith(null, expect.stringContaining('Hubcap API key is not valid'))
    );
    expect(onStatus).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });
});
