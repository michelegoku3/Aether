import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AchievementsModal } from './AchievementsModal';
import { invokeMock, mockInvokeCommands } from '../test/tauriMocks';
import type { GameAchievementsSnapshot } from '../types/achievements';

const sampleSnapshot = (overrides: Partial<GameAchievementsSnapshot> = {}): GameAchievementsSnapshot => ({
  appId: 2124490,
  gameName: 'SILENT HILL 2',
  schemaVersion: 11,
  schemaSource: 'steamCache',
  luaManaged: true,
  accountId: 920691037,
  accounts: [
    {
      accountId: 920691037,
      steamId64: '76561198880956765',
      personaName: 'MainPlayer',
      isActive: true,
      hasStatsCache: true,
    },
    {
      accountId: 123456,
      steamId64: '76561197960389184',
      personaName: 'AltAccount',
      isActive: false,
      hasStatsCache: false,
    },
  ],
  unlockedCount: 1,
  totalCount: 3,
  sources: {
    steamBinPresent: true,
    backupBinPresent: true,
    backupJsonPresent: true,
    steamUnlockCount: 1,
    backupUnlockCount: 1,
    mergedFromBackup: false,
    pendingChanges: 0,
    crc: 0,
  },
  achievements: [
    {
      id: 0,
      bucket: 0,
      bit: 0,
      apiName: 'ACH_FIRST',
      name: 'Enjoy Your Stay',
      description: 'Reach Silent Hill',
      hidden: false,
      unlocked: true,
      unlockTime: 1732660000,
      iconUrl: 'https://shared.fastly.steamstatic.com/steamcommunity/public/images/apps/2124490/first.jpg',
      iconGrayUrl: 'https://shared.fastly.steamstatic.com/steamcommunity/public/images/apps/2124490/first_gray.jpg',
      progress: null,
    },
    {
      id: 1,
      bucket: 0,
      bit: 1,
      apiName: 'ACH_COLLECTOR',
      name: 'Archivist',
      description: 'Collect all memos',
      hidden: false,
      unlocked: false,
      unlockTime: 0,
      iconUrl: 'https://shared.fastly.steamstatic.com/steamcommunity/public/images/apps/2124490/col.jpg',
      iconGrayUrl: 'https://shared.fastly.steamstatic.com/steamcommunity/public/images/apps/2124490/col_gray.jpg',
      progress: { current: 14, min: 0, max: 68 },
    },
    {
      id: 2,
      bucket: 0,
      bit: 2,
      apiName: 'ACH_SECRET',
      name: 'Maria Ending',
      description: 'See the secret ending',
      hidden: true,
      unlocked: false,
      unlockTime: 0,
      iconUrl: null,
      iconGrayUrl: null,
      progress: null,
    },
  ],
  ...overrides,
});

describe('AchievementsModal', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('separates unlocked and locked achievements, masks hidden locked ones, and displays progress', async () => {
    mockInvokeCommands({ get_game_achievements: sampleSnapshot() });
    const user = userEvent.setup();

    render(<AchievementsModal appId={2124490} gameName="SILENT HILL 2" onClose={vi.fn()} />);

    await screen.findByText('1 / 3 unlocked (33%)');
    expect(screen.getByText('Unlocked (1)')).toBeInTheDocument();
    expect(screen.getByText('Locked (2)')).toBeInTheDocument();
    expect(screen.getByText('Enjoy Your Stay')).toBeInTheDocument();
    expect(screen.getByText('14 / 68')).toBeInTheDocument();

    // Hidden locked achievement must be masked by default
    expect(screen.queryByText('Maria Ending')).not.toBeInTheDocument();
    expect(screen.getByText('Hidden achievement')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: /Reveal hidden/i }));
    expect(screen.getByText('Maria Ending')).toBeInTheDocument();
  });

  it('falls back gracefully when an achievement image fails to load', async () => {
    mockInvokeCommands({ get_game_achievements: sampleSnapshot() });
    render(<AchievementsModal appId={2124490} gameName="SILENT HILL 2" onClose={vi.fn()} />);

    const img = await screen.findByAltText('Enjoy Your Stay');
    fireEvent.error(img);
    expect(screen.queryByAltText('Enjoy Your Stay')).not.toBeInTheDocument();
  });

  it('supports search filtering, bulk unlock/lock, discard, and dirty counter', async () => {
    mockInvokeCommands({ get_game_achievements: sampleSnapshot() });
    const user = userEvent.setup();

    render(<AchievementsModal appId={2124490} gameName="SILENT HILL 2" onClose={vi.fn()} />);
    await screen.findByText('Enjoy Your Stay');

    await user.click(screen.getByRole('button', { name: 'Unlock all' }));
    expect(screen.getByText(/2 pending edit/i)).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Discard' }));
    expect(screen.queryByText(/pending edit/i)).not.toBeInTheDocument();

    await user.type(screen.getByPlaceholderText(/Search achievements/i), 'Archivist');
    expect(screen.queryByText('Enjoy Your Stay')).not.toBeInTheDocument();
    expect(screen.getByText('Archivist')).toBeInTheDocument();
  });

  it('prompts SteamCloseConfirmModal on STEAM_RUNNING and retries with allowSteamKill=true', async () => {
    let attempts = 0;
    const snap = sampleSnapshot();
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === 'get_game_achievements') {
        return snap;
      }
      if (command === 'save_game_achievements') {
        attempts += 1;
        const payload = args as { allowSteamKill: boolean };
        if (!payload.allowSteamKill) {
          throw 'STEAM_RUNNING: Steam is currently running.';
        }
        return {
          appId: 2124490,
          accountId: 920691037,
          unlockedAfter: 2,
          totalCount: 3,
          appliedUnlocks: 1,
          appliedRelocks: 0,
          luaManaged: true,
        };
      }
      return undefined;
    });

    const onStatus = vi.fn();
    const user = userEvent.setup();
    render(<AchievementsModal appId={2124490} gameName="SILENT HILL 2" onClose={vi.fn()} onStatus={onStatus} />);

    const archivistToggle = await screen.findByRole('checkbox', { name: /Archivist/i });
    await user.click(archivistToggle);
    await user.click(screen.getByRole('button', { name: 'Save changes' }));

    await screen.findByText('Close Steam?');
    // Cancelling keeps edits in memory
    await user.click(screen.getByRole('button', { name: 'No' }));
    expect(screen.getByText(/1 pending edit/i)).toBeInTheDocument();

    // Confirming closes Steam and commits
    await user.click(screen.getByRole('button', { name: 'Save changes' }));
    await screen.findByText('Close Steam?');
    await user.click(screen.getByRole('button', { name: 'Yes' }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('save_game_achievements', {
        appId: 2124490,
        accountId: 920691037,
        edits: [{ id: 1, unlocked: true }],
        allowSteamKill: true,
      })
    );
    expect(attempts).toBe(3);
    expect(onStatus).toHaveBeenCalledWith(expect.stringContaining('2/3 unlocked'), 'success');
  });

  it('shows non-Lua warning, backup merge notice, account switching, and NO_SCHEMA guidance', async () => {
    mockInvokeCommands({
      get_game_achievements: async (rawArgs?: Record<string, unknown>) => {
        const args = rawArgs as { accountId?: number | null };
        if (args.accountId === 123456) {
          throw 'NO_SCHEMA: No achievement schema cache found for AppID 2124490.';
        }
        return sampleSnapshot({
          luaManaged: false,
          sources: {
            ...sampleSnapshot().sources,
            mergedFromBackup: true,
          },
        });
      },
    });
    const user = userEvent.setup();
    render(<AchievementsModal appId={2124490} gameName="SILENT HILL 2" onClose={vi.fn()} />);

    await screen.findByText(/Not managed by Lua/i);
    expect(screen.getByText(/recovered missing unlocks from Aether backup/i)).toBeInTheDocument();

    const accountSelect = screen.getByLabelText(/Steam account/i);
    await user.selectOptions(accountSelect, '123456');

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('get_game_achievements', { appId: 2124490, accountId: 123456 })
    );
    await screen.findByText(/No achievement schema cache found/i);
  });
});
