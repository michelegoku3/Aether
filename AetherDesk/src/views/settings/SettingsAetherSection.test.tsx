import { fireEvent, render, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invokeMock } from '../../test/tauriMocks';
import { SettingsAetherSection } from './SettingsAetherSection';

const baseProps = () => ({
  enableWebviewDevtools: false,
  setEnableWebviewDevtools: vi.fn(),
  enableTestUpdates: false,
  setEnableTestUpdates: vi.fn(),
  manifestRestoreOnStartup: true,
  setManifestRestoreOnStartup: vi.fn(),
  downloadGamesWithUpdatesOn: false,
  onDownloadGamesWithUpdatesChange: vi.fn(),
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

  it('leaves the manifest-restore toggle to the form until Save', () => {
    const props = baseProps();
    invokeMock.mockReset();
    const { container } = render(<SettingsAetherSection {...props} />);

    fireEvent.click(container.querySelectorAll('input[type="checkbox"]')[2]);

    expect(props.setManifestRestoreOnStartup).toHaveBeenCalledWith(false);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('keeps the Aether group to the Aether-only switches', () => {
    const { container } = render(<SettingsAetherSection {...baseProps()} />);

    expect(container.querySelectorAll('input[type="checkbox"]')).toHaveLength(4);
    expect(container.textContent).toContain('Restore manifests on Steam startup');
    expect(container.textContent).toContain('Download games with updates on');
    expect(container.textContent).toContain('Clear AetherDesk caches');
    expect(container.textContent).not.toContain('Use OST pattern source');
    expect(container.textContent).not.toContain('Steam Installation Path');
    expect(container.textContent).not.toContain('Custom game display name');
    expect(container.textContent).not.toContain('From the library of');
  });
});

describe('SettingsAetherSection cache clearing', () => {
  beforeEach(() => {
    localStorage.clear();
    invokeMock.mockReset();
  });

  it('clears the backend and the browser caches, then reports the result', async () => {
    const props = baseProps();
    const view = render(<SettingsAetherSection {...props} />);
    localStorage.setItem('aether_cover_v4_10', '{"v":4}');
    localStorage.setItem('aether_hero_10', 'hero');
    localStorage.setItem('unrelated', 'keep');
    invokeMock.mockResolvedValueOnce('Cleared 2 cache files and 3 in-memory entries.');

    fireEvent.click(view.getByRole('button', { name: 'Clear Cache' }));

    await waitFor(() =>
      expect(props.showStatus).toHaveBeenCalledWith('Cleared 2 cache files and 3 in-memory entries.', 'success')
    );
    expect(invokeMock).toHaveBeenCalledWith('clear_app_caches');
    expect(localStorage.getItem('aether_cover_v4_10')).toBeNull();
    expect(localStorage.getItem('aether_hero_10')).toBeNull();
    expect(localStorage.getItem('unrelated')).toBe('keep');
  });

  it('does not clear the browser caches when the backend fails', async () => {
    const props = baseProps();
    const view = render(<SettingsAetherSection {...props} />);
    localStorage.setItem('aether_cover_v4_10', '{"v":4}');
    invokeMock.mockRejectedValueOnce(new Error('cache folder busy'));

    fireEvent.click(view.getByRole('button', { name: 'Clear Cache' }));

    await waitFor(() =>
      expect(props.showStatus).toHaveBeenCalledWith(expect.stringContaining('Failed to clear caches'), 'error')
    );
    expect(localStorage.getItem('aether_cover_v4_10')).toBe('{"v":4}');
  });
});
