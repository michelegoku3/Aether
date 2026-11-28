import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import { invokeMock } from '../test/tauriMocks';
import { PlayGameModal } from './PlayGameModal';
import type { RamPreview, RamProcessEntry } from '../types/play';

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

const preview = (entries: RamProcessEntry[]): RamPreview => ({
  entries,
  memory: {
    totalKb: 16 * 1024 * 1024,
    usedKb: 10 * 1024 * 1024,
    availableKb: 6 * 1024 * 1024,
  },
});

describe('PlayGameModal', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('displays system RAM overview and candidate memory in MB without dividing by 1024 twice', async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'preview_ram_optimizer') {
        return preview([
          entry({ pid: 101, name: 'chrome.exe', memoryKb: 200 * 1024 }),
          entry({ pid: 102, name: 'steam.exe', memoryKb: 300 * 1024, verdict: 'keep', reason: 'Steam' }),
        ]);
      }
      return null;
    });

    render(<PlayGameModal game={{ appId: '730', name: 'Counter-Strike 2', installed: true }} onClose={() => {}} />);

    expect(await screen.findByText(/200 MB · PID 101/)).toBeInTheDocument();
    expect(screen.getByText(/16\.0 GB used/)).toBeInTheDocument();
    expect(screen.getByText(/\+200 MB reclaimable/)).toBeInTheDocument();
  });

  it('allows Play Directly even when there are no close candidates', async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'preview_ram_optimizer') return preview([]);
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

  it('groups multi-process apps, persists kept apps, and summarizes duplicate closed names', async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'preview_ram_optimizer') {
        return preview([
          entry({ pid: 101, name: 'chrome.exe', memoryKb: 200 * 1024 }),
          entry({ pid: 102, name: 'chrome.exe', memoryKb: 250 * 1024 }),
          entry({ pid: 202, name: 'slack.exe', memoryKb: 150 * 1024 }),
          entry({ pid: 203, name: 'slack.exe', memoryKb: 150 * 1024 }),
        ]);
      }
      if (command === 'optimize_and_launch_game') {
        return {
          launched: true,
          message: 'Launched Steam app 730.',
          report: {
            closed: ['slack.exe', 'slack.exe'],
            failed: [],
            kept: ['chrome.exe (excluded by user)'],
            reclaimedKb: 300 * 1024,
            gone: [],
          },
        };
      }
      return null;
    });

    render(<PlayGameModal game={{ appId: '730', name: 'Counter-Strike 2', installed: true }} onClose={() => {}} />);

    const chromeCheckbox = await screen.findByRole('checkbox', { name: /chrome\.exe/i });
    expect(screen.getByText('450 MB')).toBeInTheDocument();
    expect(screen.getByText(/\+750 MB reclaimable/)).toBeInTheDocument();

    await user.click(chromeCheckbox);
    expect(localStorage.getItem('aether.play_keep_apps')).toBe('["chrome.exe"]');
    expect(screen.getByText(/\+300 MB reclaimable/)).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Optimize & Play' }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('optimize_and_launch_game', {
        appId: 730,
        minMemoryMb: 150,
        skipOptimize: false,
        excludePids: [101, 102],
      })
    );
    expect(await screen.findByText('×2')).toBeInTheDocument();
  });

  it('re-scans preview when the Min RAM threshold changes', async () => {
    const user = userEvent.setup();
    invokeMock.mockResolvedValue(preview([entry({ pid: 101, name: 'chrome.exe', memoryKb: 300 * 1024 })]));

    render(<PlayGameModal game={{ appId: '730', name: 'Counter-Strike 2', installed: true }} onClose={() => {}} />);

    const select = await screen.findByLabelText(/Min RAM:/i);
    await user.selectOptions(select, '250');

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('preview_ram_optimizer', {
        minMemoryMb: 250,
      })
    );
    expect(localStorage.getItem('aether.play_min_mb')).toBe('250');
  });
});
