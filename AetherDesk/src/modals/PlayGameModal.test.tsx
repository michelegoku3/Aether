import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { invokeMock } from '../test/tauriMocks';
import { PlayGameModal } from './PlayGameModal';
import type { RamProcessEntry } from '../types/play';

const entry = (overrides: Partial<RamProcessEntry>): RamProcessEntry => ({
  pid: 100,
  name: 'chrome.exe',
  exe: 'C:/chrome.exe',
  memoryKb: 200 * 1024,
  user: 'me',
  verdict: 'close',
  reason: 'memory-hungry',
  ...overrides,
});

describe('PlayGameModal', () => {
  it('displays candidate memory in MB without dividing by 1024 twice', async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'preview_ram_optimizer') {
        return [
          entry({ pid: 101, name: 'chrome.exe', memoryKb: 200 * 1024 }),
          entry({ pid: 102, name: 'steam.exe', memoryKb: 300 * 1024, verdict: 'keep', reason: 'Steam' }),
        ];
      }
      return null;
    });

    render(<PlayGameModal game={{ appId: '730', name: 'Counter-Strike 2', installed: true }} onClose={() => {}} />);

    expect(await screen.findByText(/200 MB · PID 101/)).toBeInTheDocument();
    expect(screen.getAllByText(/200 MB/).length).toBeGreaterThanOrEqual(2);
  });

  it('allows Play Directly even when there are no close candidates', async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'preview_ram_optimizer') return [];
      if (command === 'optimize_and_launch_game') {
        return {
          launched: true,
          message: 'Launched Steam app 730 directly.',
          report: { closed: [], failed: [], kept: [], reclaimedKb: 0, gone: [] },
        };
      }
      return null;
    });

    render(<PlayGameModal game={{ appId: '730', name: 'Counter-Strike 2', installed: true }} onClose={() => {}} />);

    const directBtn = await screen.findByRole('button', { name: 'Play Directly' });
    expect(directBtn).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Optimize & Play' })).toBeDisabled();

    await user.click(directBtn);

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('optimize_and_launch_game', {
        appId: 730,
        minMemoryMb: 150,
        skipOptimize: true,
        excludePids: [],
      })
    );
    expect(await screen.findByText(/Launched directly without closing background applications/)).toBeInTheDocument();
  });

  it('lets the user exclude specific processes before optimizing', async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'preview_ram_optimizer') {
        return [
          entry({ pid: 101, name: 'chrome.exe', memoryKb: 200 * 1024 }),
          entry({ pid: 202, name: 'slack.exe', memoryKb: 150 * 1024 }),
        ];
      }
      if (command === 'optimize_and_launch_game') {
        return {
          launched: true,
          message: 'Launched Steam app 730.',
          report: {
            closed: ['slack.exe'],
            failed: [],
            kept: ['chrome.exe (excluded by user)'],
            reclaimedKb: 150 * 1024,
            gone: [],
          },
        };
      }
      return null;
    });

    render(<PlayGameModal game={{ appId: '730', name: 'Counter-Strike 2', installed: true }} onClose={() => {}} />);

    const chromeCheckbox = await screen.findByRole('checkbox', { name: /chrome\.exe/i });
    expect(screen.getByText(/350 MB/)).toBeInTheDocument();

    await user.click(chromeCheckbox);
    expect(screen.getByText(/150 MB · PID 202/)).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Optimize & Play' }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('optimize_and_launch_game', {
        appId: 730,
        minMemoryMb: 150,
        skipOptimize: false,
        excludePids: [101],
      })
    );
  });
});
