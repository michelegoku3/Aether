import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { LibraryGameActionsModal } from './LibraryGameActionsModal';
import type { InstalledGame } from '../hooks/useLibraryGames';
import { invokeMock, mockInvokeCommands } from '../test/tauriMocks';

const { useLibraryGamesMock, queryGameStateMock } = vi.hoisted(() => ({
  useLibraryGamesMock: vi.fn(),
  queryGameStateMock: vi.fn(),
}));

vi.mock('../hooks/useLibraryGames', () => ({ useLibraryGames: useLibraryGamesMock }));
vi.mock('../ui/GameHeroImage', () => ({ GameHeroImage: () => null }));

const game = (id: number, state: Partial<InstalledGame> = {}): InstalledGame => ({
  id,
  name: `Game ${id}`,
  appId: String(id),
  installDir: '',
  libraryPath: '',
  gamePath: '',
  installed: false,
  installing: false,
  ...state,
});

const renderModal = (snapshot: InstalledGame, live: InstalledGame[], onStatus = vi.fn()) => {
  useLibraryGamesMock.mockReturnValue({
    games: live,
    isLoading: false,
    isRefreshing: false,
    status: { text: '', type: 'info' },
    setStatus: vi.fn(),
    loadInstalledGames: vi.fn(),
    queryGameState: queryGameStateMock,
    invalidateGameState: vi.fn(),
  });
  render(
    <LibraryGameActionsModal
      game={snapshot}
      isProcessing={false}
      onClose={vi.fn()}
      onStatus={onStatus}
      onRefresh={vi.fn()}
      onOpenVersionEditor={vi.fn()}
    />
  );
  return onStatus;
};

const onlineButton = () => screen.getByRole('button', { name: 'ONLINE' });
const removeButton = () => screen.getByRole('button', { name: 'Remove' });

describe('LibraryGameActionsModal install state', () => {
  beforeEach(() => {
    queryGameStateMock.mockReset();
  });

  it('enables ONLINE from the live list when Steam finished after the card was opened', () => {
    // The card was clicked while the game was still downloading.
    renderModal(game(100), [game(100, { installed: true, gamePath: 'C:\\Steam\\Game100' })]);

    expect(onlineButton()).toBeEnabled();
    expect(removeButton()).toBeDisabled();
  });

  it('keeps ONLINE and Remove unavailable while Steam is still installing', () => {
    renderModal(game(100, { installing: true }), [game(100, { installing: true })]);

    expect(onlineButton()).toBeDisabled();
    expect(removeButton()).toBeDisabled();
  });

  it('offers Remove only for a game Steam does not hold at all', () => {
    renderModal(game(100), [game(100)]);

    expect(onlineButton()).toBeDisabled();
    expect(removeButton()).toBeEnabled();
  });
});

describe('bundle export', () => {
  const downloadButton = () => screen.getByRole('button', { name: 'Export Lua and manifests' });

  const report = (overrides: Record<string, unknown> = {}) => ({
    path: 'C:\\Users\\me\\Downloads\\Game 100.zip',
    pins: 2,
    manifestsIncluded: 2,
    manifestsGenerated: 1,
    manifestsMissing: [] as string[],
    ...overrides,
  });

  const mockExport = (value: unknown) => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'export_game_bundle') return value;
      return undefined;
    });
  };

  beforeEach(() => {
    queryGameStateMock.mockResolvedValue(false);
  });

  it('saves the Lua and the pinned manifests under the game name', async () => {
    mockExport(report());
    const onStatus = renderModal(game(100), [game(100)]);
    const user = userEvent.setup();

    await user.click(downloadButton());

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('export_game_bundle', { appId: 100, gameName: 'Game 100' })
    );
    await waitFor(() => expect(onStatus).toHaveBeenCalledWith(expect.stringContaining('Game 100.zip'), 'success'));
  });

  it('says which pinned manifests stayed missing instead of claiming a full bundle', async () => {
    mockExport(
      report({
        manifestsIncluded: 1,
        manifestsGenerated: 0,
        manifestsMissing: ['111:1001'],
        generationError: 'no Hubcap API key is configured',
      })
    );
    const onStatus = renderModal(game(100), [game(100)]);
    const user = userEvent.setup();

    await user.click(downloadButton());

    await waitFor(() =>
      expect(onStatus).toHaveBeenCalledWith(expect.stringContaining('1 pinned manifest(s) are missing'), 'info')
    );
  });
});

describe('OFME Clean flow', () => {
  const CLEAN_CONFIRM =
    'Remove online-fix.me crack files? AetherDesk will delete the online-fix.me / SteamFix files and any UCOnline2 files it detects in the game folder. Some legitimate game files may also be removed: you can restore them by verifying the game file integrity.\n\nContinue?';
  const CLEAN_WARNING = 'Crack and UCOnline2 files were removed. Verify integrity of game files.';

  const cleanCalls: Array<{ confirm?: boolean }> = [];

  beforeEach(() => {
    cleanCalls.length = 0;
    queryGameStateMock.mockResolvedValue(false);
    invokeMock.mockImplementation(async (cmd: string, args?: { confirm?: boolean }) => {
      switch (cmd) {
        case 'get_aetheronline':
          return false;
        case 'get_online_status':
          return { state: 'not_configured', record: null };
        case 'get_aether_showonline':
          return false;
        case 'get_aether_excluded':
          return false;
        case 'get_presence_default_mode':
          return false;
        case 'inspect_foreign_online':
          return { ofme: true, uco2: true };
        case 'clean_online_conflicts':
          cleanCalls.push({ confirm: args?.confirm });
          if (args?.confirm) {
            return {
              success: true,
              message: 'Removed 3 online-fix.me / UCOnline2 file(s) from the game folder.',
              record: null,
              confirmationRequired: false,
              warning: CLEAN_WARNING,
            };
          }
          return { success: false, message: CLEAN_CONFIRM, record: null, confirmationRequired: true, warning: null };
        default:
          return undefined;
      }
    });
  });

  const openOnlineChoice = async () => {
    const user = userEvent.setup();
    renderModal(game(100, { installed: true }), [game(100, { installed: true })]);
    await user.click(onlineButton());
    return user;
  };

  it('confirms before deleting and reports the warning afterwards', async () => {
    const user = await openOnlineChoice();

    await screen.findByText(/OFME crack files detected/);
    await user.click(screen.getByRole('button', { name: 'Clean' }));

    await screen.findByText(/Remove online-fix.me crack files/);
    expect(cleanCalls).toEqual([{ confirm: false }]);

    await user.click(screen.getByRole('button', { name: 'Continue' }));

    await screen.findByText(CLEAN_WARNING);
    expect(cleanCalls).toEqual([{ confirm: false }, { confirm: true }]);

    await user.click(screen.getByText('Close'));
    await waitFor(() => expect(screen.queryByText(CLEAN_WARNING)).not.toBeInTheDocument());
  });

  it('cancelling never deletes anything', async () => {
    const user = await openOnlineChoice();

    await screen.findByText(/OFME crack files detected/);
    await user.click(screen.getByRole('button', { name: 'Clean' }));
    await screen.findByText(/Remove online-fix.me crack files/);

    await user.click(screen.getByRole('button', { name: 'Cancel' }));

    await waitFor(() => expect(screen.queryByText(/Remove online-fix.me crack files/)).not.toBeInTheDocument());
    expect(cleanCalls).toEqual([{ confirm: false }]);
  });
});

describe('online presence reads', () => {
  const presenceCalls = () => invokeMock.mock.calls.map(([command]) => command as string);

  const basePresence = {
    get_aetheronline: false,
    get_online_status: { state: 'not_configured', record: null },
    get_aether_showonline: false,
    get_aether_excluded: false,
    get_presence_default_mode: true,
    inspect_foreign_online: { ofme: false, uco2: false },
    set_aether_excluded: 'Aether now ignores this game.',
  };

  const installedGame = () => game(100, { installed: true });

  const openOnlineChoice = async () => {
    const user = userEvent.setup();
    renderModal(installedGame(), [installedGame()]);
    await user.click(onlineButton());
    await screen.findByText(/Aether ignores this game/);
    return user;
  };

  beforeEach(() => {
    queryGameStateMock.mockResolvedValue(false);
  });

  it('opening the online panel never writes presence settings', async () => {
    // The fixture has UCO2 spoofing presence and Show Online on: the panel
    // must read that state and leave the game's presence untouched.
    mockInvokeCommands({
      ...basePresence,
      get_online_status: { state: 'enabled', record: null },
      get_aether_showonline: true,
    });

    await openOnlineChoice();

    expect(presenceCalls()).not.toContain('set_aether_excluded');
    expect(presenceCalls()).toEqual(
      expect.arrayContaining([
        'get_aetheronline',
        'get_online_status',
        'get_aether_showonline',
        'get_aether_excluded',
        'get_presence_default_mode',
        'inspect_foreign_online',
      ])
    );
    // The spoof still shows as None, the mode that keeps invites working.
    expect(screen.getByRole('button', { name: /^None/ })).toHaveClass('online-choice-option--selected');
  });

  it('writes only when the user picks a mode', async () => {
    mockInvokeCommands({ ...basePresence, get_aether_showonline: true });

    const user = await openOnlineChoice();
    await user.click(screen.getByRole('button', { name: /^None/ }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('set_aether_excluded', { appId: 100, enabled: true }));
  });
});
