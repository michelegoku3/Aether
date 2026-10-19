import { fireEvent, render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { SettingsSteamSection } from './SettingsSteamSection';

const accounts = [
  { steamId: '76561199523209859', accountName: 'michelegoku3', personaName: 'michelegoku3' },
  { steamId: '76561199876393402', accountName: 'robspont15', personaName: 'SyntaxCode15' },
];

const baseProps = () => ({
  steamPath: 'C:/Steam',
  setSteamPath: vi.fn(),
  steamCheck: { state: 'idle' as const, message: '' },
  onBrowseSteamFolder: vi.fn().mockResolvedValue(undefined),
  onDetectSteamPath: vi.fn().mockResolvedValue(undefined),
  presenceDefaultShowOnline: true,
  setPresenceDefaultShowOnline: vi.fn(),
  customGameName: '',
  setCustomGameName: vi.fn(),
  libraryEnabled: false,
  setLibraryEnabled: vi.fn(),
  libraryAppIds: '',
  setLibraryAppIds: vi.fn(),
  librarySteamId: '',
  setLibrarySteamId: vi.fn(),
  steamAccounts: accounts,
  steamAccountsLoaded: true,
  steamAccountsError: false,
  showStatus: vi.fn(),
});

const text = (container: HTMLElement) => container.textContent ?? '';

describe('SettingsSteamSection', () => {
  it('holds the Steam-facing controls under one title', () => {
    const { container } = render(<SettingsSteamSection {...baseProps()} />);

    expect(container.querySelector('.settings-label')?.textContent).toBe('Steam');
    // Aether's own maintenance switches live in the Aether group.
    expect(text(container)).not.toContain('Restore manifests on Steam startup');
    expect(text(container)).not.toContain('Clear AetherDesk caches');
    expect(text(container)).toContain('Steam Installation Path');
    expect(text(container)).toContain('Browse for the Steam folder');
    expect(text(container)).toContain('Detect Steam automatically');
    expect(text(container)).toContain('Show every game to friends by default');
    expect(text(container)).toContain('Show "From the library of ..." on selected games');
  });

  it('flows straight from the title into the Steam settings, with no divider', () => {
    const { container } = render(<SettingsSteamSection {...baseProps()} />);

    expect(container.querySelectorAll('.settings-separator')).toHaveLength(0);
  });

  it('has no Library Banner title and none of the removed copy', () => {
    const { container } = render(<SettingsSteamSection {...baseProps()} libraryEnabled />);

    expect(text(container)).not.toContain('Library Banner');
    expect(text(container)).not.toContain('Comma-separated Steam App IDs');
    expect(text(container)).not.toContain('Purely cosmetic');
    expect(text(container)).not.toContain('Steam hides the banner automatically');
    expect(text(container)).not.toContain('Owner account');
  });

  it('renames the account picker to Library owner', () => {
    const { container } = render(<SettingsSteamSection {...baseProps()} libraryEnabled />);

    expect(text(container)).toContain('Library owner');
    expect(text(container)).toContain('Selected games will be shown as games from another library.');
  });

  it('shows the custom game name only while the presence toggle is on', () => {
    const on = render(<SettingsSteamSection {...baseProps()} presenceDefaultShowOnline />);
    expect(text(on.container)).toContain('Custom game display name');

    const off = render(<SettingsSteamSection {...baseProps()} presenceDefaultShowOnline={false} />);
    expect(text(off.container)).not.toContain('Custom game display name');
  });

  it('only flips the form field on the library toggle', () => {
    const props = baseProps();
    const { container } = render(<SettingsSteamSection {...props} />);

    // The library toggle is the one whose row carries the banner wording.
    const rows = Array.from(container.querySelectorAll('.settings-toggle-row'));
    const row = rows.find((r) => r.textContent?.includes('From the library of'));
    fireEvent.click(row!.querySelector('input[type="checkbox"]')!);

    expect(props.setLibraryEnabled).toHaveBeenCalledWith(true);
  });

  it('reveals the App ID input and account picker only when the banner is on', () => {
    const appIdsInput = 'input[placeholder*="4656000"]';
    const off = render(<SettingsSteamSection {...baseProps()} />);
    expect(off.container.querySelector(appIdsInput)).toBeNull();
    expect(off.container.querySelector('select')).toBeNull();

    const on = render(<SettingsSteamSection {...baseProps()} libraryEnabled />);
    expect(on.container.querySelector(appIdsInput)).not.toBeNull();
    expect(on.container.querySelector('select')).not.toBeNull();
  });
});
