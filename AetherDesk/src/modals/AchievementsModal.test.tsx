import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
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
      iconUrl: 'https://cdn.fastly.steamstatic.com/steamcommunity/public/images/apps/2124490/first.jpg',
      iconGrayUrl: 'https://cdn.fastly.steamstatic.com/steamcommunity/public/images/apps/2124490/first_gray.jpg',
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
      iconUrl: 'https://cdn.fastly.steamstatic.com/steamcommunity/public/images/apps/2124490/col.jpg',
      iconGrayUrl: 'https://cdn.fastly.steamstatic.com/steamcommunity/public/images/apps/2124490/col_gray.jpg',
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

  it('uses bold title format, omits schema/pending footer text and tabs, and shows section titles without Hidden badge', async () => {
    mockInvokeCommands({ get_game_achievements: sampleSnapshot() });
    const user = userEvent.setup();

    render(<AchievementsModal appId={2124490} gameName="SILENT HILL 2" onClose={vi.fn()} />);

    await screen.findByText('1 / 3 unlocked (33%)');

    const titleEl = document.querySelector('.modal-title');
    expect(titleEl?.textContent).toBe('Achievements: SILENT HILL 2 (2124490)');
    expect(titleEl?.querySelector('strong')?.textContent).toBe('SILENT HILL 2');

    expect(screen.getByText('Unlocked Achievements (1)')).toBeInTheDocument();
    expect(screen.getByText('Locked Achievements (2)')).toBeInTheDocument();
    expect(screen.getByText('14 / 68')).toBeInTheDocument();

    // No filter tabs, no schema meta, no pending count, no standalone "Hidden" badge
    expect(screen.queryByRole('tab')).not.toBeInTheDocument();
    expect(screen.queryByText(/Schema v/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/^Hidden$/)).not.toBeInTheDocument();

    // Hidden locked achievement stays masked until "Show Hidden" is clicked
    expect(screen.queryByText('Maria Ending')).not.toBeInTheDocument();
    expect(screen.getByText('Hidden achievement')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Show Hidden' }));
    expect(screen.getByText('Maria Ending')).toBeInTheDocument();
  });

  it('keeps toggled achievements in their original section and updates badges in place', async () => {
    mockInvokeCommands({ get_game_achievements: sampleSnapshot() });
    const user = userEvent.setup();

    render(<AchievementsModal appId={2124490} gameName="SILENT HILL 2" onClose={vi.fn()} />);
    await screen.findByText('Archivist');

    const lockedHeading = screen.getByText('Locked Achievements (2)');
    const lockedSection = lockedHeading.closest('section')!;
    const archivistToggle = within(lockedSection).getByRole('checkbox', { name: /Archivist/i });

    await user.click(archivistToggle);

    // Archivist stays inside Locked Achievements with a "Will unlock" badge
    expect(within(lockedSection).getByText('Archivist')).toBeInTheDocument();
    expect(within(lockedSection).getByText('Will unlock')).toBeInTheDocument();
    expect(screen.getByText('Locked Achievements (2)')).toBeInTheDocument();
    expect(screen.getByText('Unlocked Achievements (1)')).toBeInTheDocument();
    expect(screen.queryByText(/pending edit/i)).not.toBeInTheDocument();

    // Re-lock a previously unlocked item: stays inside Unlocked Achievements with "Will relock"
    const unlockedSection = screen.getByText('Unlocked Achievements (1)').closest('section')!;
    await user.click(within(unlockedSection).getByRole('checkbox', { name: /Enjoy Your Stay/i }));
    expect(within(unlockedSection).getByText('Enjoy Your Stay')).toBeInTheDocument();
    expect(within(unlockedSection).getByText('Will relock')).toBeInTheDocument();
  });

  it('tries fallback Steam CDNs before falling back to TrophyIcon', async () => {
    mockInvokeCommands({ get_game_achievements: sampleSnapshot() });
    render(<AchievementsModal appId={2124490} gameName="SILENT HILL 2" onClose={vi.fn()} />);

    const img = (await screen.findByAltText('Enjoy Your Stay')) as HTMLImageElement;
    expect(img.src).toContain('cdn.fastly.steamstatic.com');

    fireEvent.error(img);
    await waitFor(() => {
      const retryImg = screen.getByAltText('Enjoy Your Stay') as HTMLImageElement;
      expect(retryImg.src).toContain('cdn.akamai.steamstatic.com');
    });

    fireEvent.error(screen.getByAltText('Enjoy Your Stay'));
    await waitFor(() => {
      const retryImg2 = screen.getByAltText('Enjoy Your Stay') as HTMLImageElement;
      expect(retryImg2.src).toContain('steamcdn-a.akamaihd.net');
    });

    fireEvent.error(screen.getByAltText('Enjoy Your Stay'));
    await waitFor(() => {
      const retryImg3 = screen.getByAltText('Enjoy Your Stay') as HTMLImageElement;
      expect(retryImg3.src).toContain('media.steampowered.com');
    });

    fireEvent.error(screen.getByAltText('Enjoy Your Stay'));
    expect(screen.queryByAltText('Enjoy Your Stay')).not.toBeInTheDocument();
  });

  it('keeps only 3 toolbar buttons, swaps Unlock All / Lock All to Discard when clicked, and filters via search', async () => {
    mockInvokeCommands({ get_game_achievements: sampleSnapshot() });
    const user = userEvent.setup();

    render(<AchievementsModal appId={2124490} gameName="SILENT HILL 2" onClose={vi.fn()} />);
    await screen.findByText('Enjoy Your Stay');

    const toolbar = document.querySelector('.ach-toolbar')!;
    const buttons = within(toolbar as HTMLElement).getAllByRole('button');
    expect(buttons.map((b) => b.textContent)).toEqual(['Show Hidden', 'Unlock All', 'Lock All']);

    const searchInput = screen.getByPlaceholderText(/Search achievements/i);
    expect(searchInput).toHaveClass('library-search-input');

    // Click Unlock All -> turns into Discard, Save becomes enabled
    await user.click(screen.getByRole('button', { name: 'Unlock All' }));
    expect(screen.getByRole('button', { name: 'Discard' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save' })).toBeEnabled();

    // Clicking Discard reverts pending edits and restores Unlock All
    await user.click(screen.getByRole('button', { name: 'Discard' }));
    expect(screen.getByRole('button', { name: 'Unlock All' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();

    // Search filters visible items
    await user.type(searchInput, 'Archivist');
    expect(screen.queryByText('Enjoy Your Stay')).not.toBeInTheDocument();
    expect(screen.getByText('Archivist')).toBeInTheDocument();
  });

  it('closes only via X or Close button, ignoring overlay click and Escape key', async () => {
    mockInvokeCommands({ get_game_achievements: sampleSnapshot() });
    const onClose = vi.fn();
    const user = userEvent.setup();

    render(<AchievementsModal appId={2124490} gameName="SILENT HILL 2" onClose={onClose} />);
    await screen.findByText('Enjoy Your Stay');

    await user.click(document.querySelector('.modal-overlay')!);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).not.toHaveBeenCalled();

    await user.click(screen.getByText('Close'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('prompts SteamCloseConfirmModal on STEAM_RUNNING when Save is clicked and retries with allowSteamKill=true', async () => {
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
    await user.click(screen.getByRole('button', { name: 'Save' }));

    await screen.findByText('Close Steam?');
    // Cancelling keeps edits
    await user.click(screen.getByRole('button', { name: 'No' }));
    expect(screen.getByRole('button', { name: 'Save' })).toBeEnabled();

    // Confirming closes Steam and commits
    await user.click(screen.getByRole('button', { name: 'Save' }));
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
