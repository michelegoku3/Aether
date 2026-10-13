import { fireEvent, render } from '@testing-library/react';
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
  showStatus: vi.fn(),
});

describe('SettingsAetherSection devtools toggle', () => {
  it('only flips the form field: opening happens on Save, never on toggle', () => {
    const props = baseProps();
    const { container } = render(<SettingsAetherSection {...props} />);

    fireEvent.click(container.querySelectorAll('input[type="checkbox"]')[0]);

    expect(props.setEnableWebviewDevtools).toHaveBeenCalledWith(true);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('toggling off only flips the form field as well', () => {
    const props = { ...baseProps(), enableWebviewDevtools: true };
    const { container } = render(<SettingsAetherSection {...props} />);

    fireEvent.click(container.querySelectorAll('input[type="checkbox"]')[0]);

    expect(props.setEnableWebviewDevtools).toHaveBeenCalledWith(false);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
