import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invokeMock, mockInvokeCommands } from '../test/tauriMocks';
import { AetherView } from './AetherView';

const baseProps = () => ({
  isUpdateAvailable: false,
  isDeskUpdateAvailable: false,
  deskVersion: '1.4.3',
  isDllUpdateTest: false,
  isDeskUpdateTest: false,
  onUpdateComplete: vi.fn(),
  dllStatus: { isInstalled: true, installedVersion: '1.0.0', isSteamBlocked: false },
  onDllStatusChange: vi.fn().mockResolvedValue(undefined),
});

/** Opens the confirm popup (deleteUserData unchecked) and advances until the
 *  residuals probe has decided the next step. */
const openUninstallConfirmAndConfirm = async (residualCount: number, extraCommands: Record<string, unknown> = {}) => {
  mockInvokeCommands({ get_settings: {}, probe_aether_steam_residuals: residualCount, ...extraCommands });
  render(<AetherView {...baseProps()} />);

  const deskPanel = screen.getByText('AetherDesk', { selector: '.panel-title' }).closest('.aether-panel');
  expect(deskPanel).not.toBeNull();
  await act(async () => {}); // let getSettings resolve

  fireEvent.click(within(deskPanel as HTMLElement).getByRole('button', { name: 'Uninstall' }));
  expect(screen.getByText('Are you sure you want to uninstall AetherDesk?')).toBeInTheDocument();

  fireEvent.click(screen.getByRole('button', { name: 'Yes' }));
  await act(async () => {}); // flush the residuals probe
};

describe('AetherView uninstall flow', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows the Reset Steam path confirm when Steam residuals exist, then resets and uninstalls', async () => {
    await openUninstallConfirmAndConfirm(3, {
      reset_aether_steam_path: 'ok',
      unblock_steam_updates: 'ok',
      uninstall_aether_desk: 'ok',
    });

    // The old Steam-clean prompt is gone: its title must not be reachable.
    expect(screen.queryByText('Clean Steam first?')).not.toBeInTheDocument();

    // The same gate used by the Reset Path button takes over, countdown included.
    expect(screen.getByText('Resetting the Steam path cannot be undone.')).toBeInTheDocument();
    const yesButton = screen.getByRole('button', { name: /Yes, clean Steam/ });
    expect(yesButton).toBeDisabled();
    expect(yesButton).toHaveTextContent('Yes, clean Steam (10)');

    for (let elapsed = 0; elapsed < 10; elapsed += 1) {
      act(() => {
        vi.advanceTimersByTime(1000);
      });
    }
    expect(yesButton).toBeEnabled();

    await act(async () => {
      fireEvent.click(yesButton);
    });
    await act(async () => {}); // drain the reset + unblock + uninstall chain

    expect(invokeMock).toHaveBeenCalledWith('probe_aether_steam_residuals');
    expect(invokeMock).toHaveBeenCalledWith('reset_aether_steam_path');
    expect(invokeMock).toHaveBeenCalledWith('uninstall_aether_desk', { deleteUserData: false });
  });

  it('lets NO on the reset confirm skip the Steam clean and just uninstall', async () => {
    await openUninstallConfirmAndConfirm(3, { uninstall_aether_desk: 'ok' });

    // In the uninstall flow NO means "skip the clean, still remove AetherDesk".
    fireEvent.click(screen.getByRole('button', { name: 'No, just uninstall' }));
    await act(async () => {}); // drain the uninstall chain

    expect(invokeMock).not.toHaveBeenCalledWith('reset_aether_steam_path');
    expect(invokeMock).toHaveBeenCalledWith('uninstall_aether_desk', { deleteUserData: false });
  });

  it('lets the X dismiss abort the whole uninstall without touching disk', async () => {
    await openUninstallConfirmAndConfirm(3);

    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    await act(async () => {});

    expect(screen.queryByText(/Reset Steam path/)).not.toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith('reset_aether_steam_path');
    expect(invokeMock).not.toHaveBeenCalledWith('uninstall_aether_desk', { deleteUserData: false });
  });
});
