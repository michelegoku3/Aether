import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { emitTauriEvent, invokeMock, mockInvokeCommands } from '../test/tauriMocks';
import { DESK_UPDATE_PROGRESS_EVENT } from '../constants/updates';
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
  mockInvokeCommands({
    get_settings: {},
    probe_aether_steam_residuals: residualCount,
    // Steam closed by default: only the tests that open the Steam gate pass true.
    probe_steam_running: false,
    ...extraCommands,
  });
  render(<AetherView {...baseProps()} />);

  const deskPanel = screen.getByText('AetherDesk', { selector: '.panel-title' }).closest('.aether-panel');
  expect(deskPanel).not.toBeNull();
  await act(async () => {}); // let getSettings resolve

  fireEvent.click(within(deskPanel as HTMLElement).getByRole('button', { name: 'Uninstall' }));
  expect(screen.getByText('Are you sure you want to uninstall AetherDesk?')).toBeInTheDocument();

  fireEvent.click(screen.getByRole('button', { name: 'Yes' }));
  await act(async () => {}); // flush the residuals probe
};

describe('AetherView desk update progress', () => {
  it('shows one bar for the whole update and no percentage in the text', async () => {
    mockInvokeCommands({
      get_settings: {},
      // Stays pending on purpose: a successful update exits the app, so the
      // promise never resolves in real life either.
      install_aether_desk_update: new Promise(() => {}),
    });
    const { container } = render(<AetherView {...baseProps()} isDeskUpdateAvailable />);
    await act(async () => {}); // let getSettings resolve

    const deskPanel = screen
      .getByText('AetherDesk', { selector: '.panel-title' })
      .closest('.aether-panel') as HTMLElement;
    fireEvent.click(within(deskPanel).getByRole('button', { name: 'Update' }));
    await act(async () => {}); // listener attached

    act(() => {
      emitTauriEvent(DESK_UPDATE_PROGRESS_EVENT, {
        phase: 'downloading',
        downloadedBytes: 45,
        totalBytes: 180,
      });
    });

    expect(screen.getByText('Downloading AetherDesk (0s)')).toBeInTheDocument();
    expect(screen.queryByText(/%/)).not.toBeInTheDocument();
    // A quarter of the download fills the 3-75 band: 21 % of the single bar.
    expect(container.querySelector('.version-progress-fill')).toHaveStyle({ width: '21%' });
    // The bar, not the text, is what assistive tech reads for progress.
    expect(screen.getByRole('progressbar', { name: 'AetherDesk update' })).toHaveAttribute('aria-valuenow', '21');
    // The label sits above the bar; the column order follows the DOM.
    expect(container.querySelector('.aether-update-progress')?.firstElementChild).toHaveClass('version-progress-label');
  });

  it('keeps a preparing line after the status alert expires', async () => {
    vi.useFakeTimers();
    mockInvokeCommands({
      get_settings: {},
      install_aether_desk_update: new Promise(() => {}),
    });
    render(<AetherView {...baseProps()} isDeskUpdateAvailable />);
    await act(async () => {});

    const deskPanel = screen
      .getByText('AetherDesk', { selector: '.panel-title' })
      .closest('.aether-panel') as HTMLElement;
    fireEvent.click(within(deskPanel).getByRole('button', { name: 'Update' }));
    await act(async () => {});

    // The status alert clears after 6 s; the update is still running.
    act(() => {
      vi.advanceTimersByTime(7_000);
    });

    expect(screen.getAllByText('Preparing AetherDesk portable update...')).toHaveLength(1);
    vi.useRealTimers();
  });
});

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

    // The Steam-clean prompt is not used by this flow: its title must not be
    // reachable.
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
    expect(invokeMock).toHaveBeenCalledWith('reset_aether_steam_path', { allowSteamKill: false });
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

const STEAM_CLOSE_LEAD = 'This operation will close Steam. Are you sure you want to do it?';

describe('AetherView Steam close confirm', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  const aetherDllPanel = () =>
    screen.getByText('AetherDLL', { selector: '.panel-title' }).closest('.aether-panel') as HTMLElement;

  const renderView = async (commands: Record<string, unknown>, dllInstalled = true) => {
    mockInvokeCommands({ get_settings: {}, ...commands });
    render(
      <AetherView
        {...baseProps()}
        dllStatus={{
          isInstalled: dllInstalled,
          installedVersion: dllInstalled ? '1.0.0' : 'N/A',
          isSteamBlocked: false,
        }}
      />
    );
    await act(async () => {}); // let getSettings resolve
  };

  it('asks before closing Steam and leaves the DLLs alone on NO', async () => {
    await renderView({ probe_steam_running: true }, false);

    fireEvent.click(within(aetherDllPanel()).getByRole('button', { name: 'Install' }));
    await act(async () => {});

    expect(screen.getByText(STEAM_CLOSE_LEAD)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'No' }));
    await act(async () => {});

    expect(screen.queryByText(STEAM_CLOSE_LEAD)).not.toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith('install_aether_dll', expect.anything());
  });

  it('installs with the Steam kill allowed after YES', async () => {
    await renderView({ probe_steam_running: true, install_aether_dll: 'installed' }, false);

    fireEvent.click(within(aetherDllPanel()).getByRole('button', { name: 'Install' }));
    await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: 'Yes' }));
    await act(async () => {});

    expect(invokeMock).toHaveBeenCalledWith('install_aether_dll', { allowSteamKill: true });
    expect(screen.queryByText(STEAM_CLOSE_LEAD)).not.toBeInTheDocument();
  });

  it('skips the popup when Steam is closed', async () => {
    await renderView({ probe_steam_running: false, install_aether_dll: 'installed' }, false);

    fireEvent.click(within(aetherDllPanel()).getByRole('button', { name: 'Install' }));
    await act(async () => {});

    expect(screen.queryByText(STEAM_CLOSE_LEAD)).not.toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith('install_aether_dll', { allowSteamKill: false });
  });

  it('closes Steam before removing the DLLs', async () => {
    await renderView({ probe_steam_running: true, uninstall_aether_dll: 'removed' });

    fireEvent.click(within(aetherDllPanel()).getByRole('button', { name: 'Uninstall' }));
    await act(async () => {});
    expect(screen.getByText(STEAM_CLOSE_LEAD)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Yes' }));
    await act(async () => {});

    expect(invokeMock).toHaveBeenCalledWith('uninstall_aether_dll', { allowSteamKill: true });
  });

  it('closes Steam only after the Reset Path countdown and its own YES', async () => {
    await renderView({ probe_steam_running: true, reset_aether_steam_path: 'ok', unblock_steam_updates: 'ok' });

    fireEvent.click(screen.getByRole('button', { name: 'Reset Path' }));
    for (let elapsed = 0; elapsed < 10; elapsed += 1) {
      act(() => {
        vi.advanceTimersByTime(1000);
      });
    }
    fireEvent.click(screen.getByRole('button', { name: 'Yes, clean Steam' }));
    await act(async () => {});

    expect(screen.getByText(STEAM_CLOSE_LEAD)).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith('reset_aether_steam_path', expect.anything());

    fireEvent.click(screen.getByRole('button', { name: 'Yes' }));
    await act(async () => {});

    expect(invokeMock).toHaveBeenCalledWith('reset_aether_steam_path', { allowSteamKill: true });
    expect(invokeMock).toHaveBeenCalledWith('unblock_steam_updates');
  });

  it('skips the Steam clean but still uninstalls AetherDesk on NO', async () => {
    await openUninstallConfirmAndConfirm(3, { probe_steam_running: true, uninstall_aether_desk: 'ok' });

    for (let elapsed = 0; elapsed < 10; elapsed += 1) {
      act(() => {
        vi.advanceTimersByTime(1000);
      });
    }
    fireEvent.click(screen.getByRole('button', { name: 'Yes, clean Steam' }));
    await act(async () => {});
    expect(screen.getByText(STEAM_CLOSE_LEAD)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'No' }));
    await act(async () => {});

    expect(invokeMock).not.toHaveBeenCalledWith('reset_aether_steam_path', expect.anything());
    expect(invokeMock).toHaveBeenCalledWith('uninstall_aether_desk', { deleteUserData: false });
  });

  it('closes Steam, cleans the path and uninstalls on YES', async () => {
    await openUninstallConfirmAndConfirm(3, {
      probe_steam_running: true,
      reset_aether_steam_path: 'ok',
      unblock_steam_updates: 'ok',
      uninstall_aether_desk: 'ok',
    });

    for (let elapsed = 0; elapsed < 10; elapsed += 1) {
      act(() => {
        vi.advanceTimersByTime(1000);
      });
    }
    fireEvent.click(screen.getByRole('button', { name: 'Yes, clean Steam' }));
    await act(async () => {});
    expect(screen.getByText(STEAM_CLOSE_LEAD)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Yes' }));
    await act(async () => {});

    expect(invokeMock).toHaveBeenCalledWith('reset_aether_steam_path', { allowSteamKill: true });
    expect(invokeMock).toHaveBeenCalledWith('unblock_steam_updates');
    expect(invokeMock).toHaveBeenCalledWith('uninstall_aether_desk', { deleteUserData: false });
    expect(screen.queryByText(STEAM_CLOSE_LEAD)).not.toBeInTheDocument();
  });

  it('lets X on the Steam popup abort and re-enable the panel', async () => {
    await renderView({ probe_steam_running: true, install_aether_dll: 'installed' }, false);
    const installButton = within(aetherDllPanel()).getByRole('button', { name: 'Install' });

    fireEvent.click(installButton);
    await act(async () => {});
    expect(screen.getByText(STEAM_CLOSE_LEAD)).toBeInTheDocument();
    expect(installButton).toBeDisabled();

    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    await act(async () => {});

    expect(screen.queryByText(STEAM_CLOSE_LEAD)).not.toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith('install_aether_dll', expect.anything());
    expect(installButton).toBeEnabled();
  });

  it('starts a single operation when the click is repeated during the probe', async () => {
    let answer: ((value: boolean) => void) | undefined;
    const pendingProbe = new Promise<boolean>((resolve) => {
      answer = resolve;
    });
    await renderView({ probe_steam_running: pendingProbe, install_aether_dll: 'installed' }, false);
    const installButton = within(aetherDllPanel()).getByRole('button', { name: 'Install' });

    // Both clicks land before React can re-render the disabled state: the guard
    // has to hold on its own.
    await act(async () => {
      installButton.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      installButton.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      answer?.(false);
    });
    await act(async () => {});

    expect(invokeMock.mock.calls.filter(([command]) => command === 'install_aether_dll')).toHaveLength(1);
  });
});
