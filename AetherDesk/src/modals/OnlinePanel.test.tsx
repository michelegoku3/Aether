import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { OnlinePanel } from './OnlinePanel';
import type { LibraryActionGame } from './LibraryGameActionsModal';
import { invokeMock } from '../test/tauriMocks';

const CONFIRM_TEXT =
  'No recovery journal found. AetherDesk can remove the known UCOnline2 files, but some legitimate game files may also be removed. You can easily restore them by verifying the game file integrity.\n\nContinue?';
const WARNING_TEXT =
  'UCO2 files were removed. Some legitimate game files may also be removed. You can easily restore them by verifying the game file integrity: Steam > Library > Verify integrity of game files.';

const game: LibraryActionGame = {
  id: 100,
  name: 'Game 100',
  appId: '100',
  installDir: '',
  libraryPath: '',
  gamePath: 'C:\\Game',
  installed: true,
  installing: false,
};

const plan = {
  detection: {
    gameRoot: 'C:\\Game',
    engine: 'unity' as const,
    arch: 'x64' as const,
    gameExe: 'C:\\Game\\Game.exe',
    unityDataDir: null,
    steamApiDir: 'C:\\Game',
    iniDir: 'C:\\Game',
    backends: { photon: 'none' as const, photonVoice: false, eos: false, playfab: false, coherence: false },
    conflicts: [],
    steamlessApplied: false,
    steamstubDetected: false,
    warnings: [],
  },
  prerequisites: { bundleOk: true, bundleVersion: '1.0', steamApiDirWritable: true, errors: [] },
  current: null,
  notices: [],
};

const enabledStatus = {
  state: 'enabled' as const,
  record: {
    appId: 100,
    enabledAt: 0,
    bundleVersion: '1.0',
    ogAppId: 100,
    spoofAppId: 480,
    iniPath: 'C:\\Game\\union-crax.ini',
    steamApiPath: 'C:\\Game\\steam_api64.dll',
    arch: 'x64' as const,
    backendsDeployed: [],
    backupDir: 'C:\\Data\\backup\\100',
    overlayProxyPath: null,
  },
};

const disableCalls: Array<{ confirmCleanup?: boolean }> = [];

beforeEach(() => {
  disableCalls.length = 0;
  invokeMock.mockImplementation(async (cmd: string, args?: { confirmCleanup?: boolean }) => {
    switch (cmd) {
      case 'plan_online':
        return plan;
      case 'get_online_status':
        return enabledStatus;
      case 'get_online_preferences':
        return null;
      case 'disable_online':
        disableCalls.push({ confirmCleanup: args?.confirmCleanup });
        if (args?.confirmCleanup) {
          return {
            success: true,
            message: 'Online disabled: UCOnline2 files removed without a recovery journal.',
            record: null,
            confirmationRequired: false,
            warning: WARNING_TEXT,
          };
        }
        return { success: false, message: CONFIRM_TEXT, record: null, confirmationRequired: true, warning: null };
      default:
        throw new Error(`unexpected command ${cmd}`);
    }
  });
});

describe('OnlinePanel journal-less disable', () => {
  it('confirms before cleaning and reports the verify-integrity warning', async () => {
    const user = userEvent.setup();
    render(<OnlinePanel game={game} onClose={vi.fn()} />);

    const disable = await screen.findByRole('button', { name: 'Disable' });
    await user.click(disable);

    // First call asks for confirmation and deletes nothing.
    await screen.findByText(/No recovery journal found/);
    expect(disableCalls).toEqual([{ confirmCleanup: false }]);

    await user.click(screen.getByRole('button', { name: 'Continue' }));

    // Second call performs the cleanup and surfaces the warning popup.
    await screen.findByText(WARNING_TEXT);
    expect(disableCalls).toEqual([{ confirmCleanup: false }, { confirmCleanup: true }]);

    await user.click(screen.getByText('Close'));
    await waitFor(() => expect(screen.queryByText(WARNING_TEXT)).not.toBeInTheDocument());
  });

  it('cancelling the confirmation keeps the panel and never cleans up', async () => {
    const user = userEvent.setup();
    render(<OnlinePanel game={game} onClose={vi.fn()} />);

    await user.click(await screen.findByRole('button', { name: 'Disable' }));
    await screen.findByText(/No recovery journal found/);

    await user.click(screen.getByRole('button', { name: 'Cancel' }));

    await waitFor(() => expect(screen.queryByText(/No recovery journal found/)).not.toBeInTheDocument());
    expect(disableCalls).toEqual([{ confirmCleanup: false }]);
    expect(screen.getByRole('button', { name: 'Disable' })).toBeEnabled();
  });
});
