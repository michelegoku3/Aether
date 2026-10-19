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
  manifestRestoreOnStartup: true,
  setManifestRestoreOnStartup: vi.fn(),
  downloadGamesWithUpdatesOn: false,
  onDownloadGamesWithUpdatesChange: vi.fn(),
  workshopAutoDownloadContent: true,
  setWorkshopAutoDownloadContent: vi.fn(),
  ostWarningAcknowledged: false,
  setShowOstWarning: vi.fn(),
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

  it('keeps the Aether group to the Aether-only switches', () => {
    const { container } = render(<SettingsAetherSection {...baseProps()} />);

    expect(container.querySelectorAll('input[type="checkbox"]')).toHaveLength(6);
    expect(container.textContent).toContain('Restore manifests on Steam startup');
    expect(container.textContent).toContain('Download games with updates on');
    expect(container.textContent).toContain('Workshop auto-download content');
    expect(container.textContent).toContain('Clear AetherDesk caches');
    expect(container.textContent).not.toContain('Steam Installation Path');
    expect(container.textContent).not.toContain('Custom game display name');
    expect(container.textContent).not.toContain('From the library of');
  });
});
