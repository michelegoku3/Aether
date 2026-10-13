import { fireEvent, render, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { invokeMock } from '../../test/tauriMocks';
import { SettingsAetherSection } from './SettingsAetherSection';

const baseProps = () => ({
  enableWebviewDevtools: false,
  setEnableWebviewDevtools: vi.fn(),
  enableTestUpdates: false,
  setEnableTestUpdates: vi.fn(),
  useOstSource: false,
  setUseOstSource: vi.fn(),
  ostWarningAcknowledged: false,
  setShowOstWarning: vi.fn(),
  manifestRestoreOnStartup: true,
  setManifestRestoreOnStartup: vi.fn(),
  presenceDefaultShowOnline: true,
  setPresenceDefaultShowOnline: vi.fn(),
  customGameName: '',
  setCustomGameName: vi.fn(),
  steamPath: '',
  setSteamPath: vi.fn(),
  steamCheck: { state: 'idle' as const, message: '' },
  onBrowseSteamFolder: vi.fn().mockResolvedValue(undefined),
  onDetectSteamPath: vi.fn().mockResolvedValue(undefined),
  persistDevtools: vi.fn().mockResolvedValue(undefined),
  showStatus: vi.fn(),
});

describe('SettingsAetherSection devtools gate', () => {
  it('persists the opt-in before opening devtools', async () => {
    const order: string[] = [];
    const props = baseProps();
    props.persistDevtools.mockImplementation(async () => { order.push('persist'); });
    invokeMock.mockImplementation(async () => { order.push('open'); return undefined; });
    const { container } = render(<SettingsAetherSection {...props} />);

    fireEvent.click(container.querySelectorAll('input[type="checkbox"]')[0]);

    await waitFor(() => expect(order).toEqual(['persist', 'open']));
    expect(props.setEnableWebviewDevtools).toHaveBeenCalledWith(true);
  });

  it('shows a status error when opening fails', async () => {
    const props = baseProps();
    invokeMock.mockRejectedValueOnce(new Error('devtools disabled'));
    const { container } = render(<SettingsAetherSection {...props} />);

    fireEvent.click(container.querySelectorAll('input[type="checkbox"]')[0]);

    await waitFor(() => expect(props.showStatus).toHaveBeenCalledWith(
      expect.stringContaining('Unable to open WebView devtools'),
      'error',
    ));
  });
});
