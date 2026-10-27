import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { openInFileManager } from '../../util/paths';
import { SettingsAppearanceSection } from './SettingsAppearanceSection';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue('themes') }));
vi.mock('../../util/paths', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../util/paths')>()),
  openInFileManager: vi.fn().mockResolvedValue(undefined),
}));

const pickButton = (label: string, onClick: () => void, disabled: boolean, busy: boolean) => (
  <button type="button" key={label} className="appearance-pick-btn" onClick={onClick} disabled={disabled || busy}>
    {busy ? '...' : label}
  </button>
);

const baseProps = () => ({
  appearanceAssets: {
    themeExists: true,
    themeName: 'cyberpunk.css',
    wallpaperExists: true,
    wallpaperName: 'cyberpunk.jpg',
    iconExists: true,
    iconName: 'aether.ico',
    themesDir: 'C:/AetherData/config/themes',
    wallpapersDir: 'C:/AetherData/config/wallpapers',
    iconsDir: 'C:/AetherData/config/icons',
    themeKitDir: 'C:/AetherData/config/theme-kit',
  },
  customCssEnabled: true,
  setCustomCssEnabled: vi.fn(),
  onCustomCssChange: vi.fn(),
  handlePickTheme: vi.fn(),
  isPicking: null,
  personalWallpaperEnabled: true,
  setPersonalWallpaperEnabled: vi.fn(),
  onPreviewPersonalWallpaper: vi.fn(),
  handlePickWallpaper: vi.fn(),
  personalWallpaperOpacity: 20,
  setPersonalWallpaperOpacity: vi.fn(),
  customIconEnabled: true,
  setCustomIconEnabled: vi.fn(),
  handlePickIcon: vi.fn(),
  useAlternativeGameCards: false,
  setUseAlternativeGameCards: vi.fn(),
  alternativeCardsOpacity: 100,
  setAlternativeCardsOpacity: vi.fn(),
  alternativeCardsFade: 50,
  setAlternativeCardsFade: vi.fn(),
  onPreviewAlternativeCards: vi.fn(),
  setIconSelectedFile: vi.fn(),
  showStatus: vi.fn(),
  appearancePickBtn: pickButton,
});

describe('SettingsAppearanceSection theme buttons', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockClear();
    vi.mocked(openInFileManager).mockClear();
  });

  it('gives the kit its own row under the theme with the stylesheet explanation', () => {
    render(<SettingsAppearanceSection {...baseProps()} />);

    const theme = screen.getByRole('button', { name: 'Theme' });
    const kit = screen.getByRole('button', { name: 'Kit' });
    expect(screen.getByRole('button', { name: 'Wallpaper' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Icon' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'THEME' })).not.toBeInTheDocument();
    expect(theme.compareDocumentPosition(kit) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    const themeRow = theme.closest('.settings-appearance-row');
    expect(themeRow?.textContent).toContain('Currently active theme: cyberpunk.css.');
    expect(themeRow?.textContent).not.toContain('is applied automatically');
    expect(themeRow?.textContent).not.toContain('Use the button');

    const kitRow = kit.closest('.settings-appearance-row');
    expect(kitRow).not.toBe(themeRow);
    expect(kitRow?.textContent).toContain(
      'default.css is the stylesheet that AetherDesk uses, copy it and give it to an AI to build your own theme.'
    );
    expect(screen.queryByText('Theme kit')).not.toBeInTheDocument();
  });

  it('leaves only the active wallpaper and icon names in their picker rows', () => {
    render(<SettingsAppearanceSection {...baseProps()} />);

    const wallpaperRow = screen.getByRole('button', { name: 'Wallpaper' }).closest('.settings-appearance-row');
    const iconRow = screen.getByRole('button', { name: 'Icon' }).closest('.settings-appearance-row');
    expect(wallpaperRow?.textContent).toContain('Currently active wallpaper: cyberpunk.jpg.');
    expect(iconRow?.textContent).toContain('Currently active icon: aether.ico.');
    expect(screen.queryByText(/The first image in/)).not.toBeInTheDocument();
    expect(screen.queryByText(/The first icon in/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Use the button to pick/)).not.toBeInTheDocument();
  });

  it('hides the kit while the custom theme is off', () => {
    render(<SettingsAppearanceSection {...baseProps()} customCssEnabled={false} />);

    expect(screen.queryByRole('button', { name: 'Kit' })).not.toBeInTheDocument();
  });

  it('opens the theme kit folder after ensuring it exists', async () => {
    render(<SettingsAppearanceSection {...baseProps()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Kit' }));

    expect(invoke).toHaveBeenCalledWith('ensure_custom_css');
    await vi.waitFor(() => expect(openInFileManager).toHaveBeenCalledWith('C:/AetherData/config/theme-kit'));
  });

  it('leaves the custom-icon switch to the form until Save', async () => {
    const props = baseProps();
    const { container } = render(<SettingsAppearanceSection {...props} />);

    const row = Array.from(container.querySelectorAll('.settings-toggle-row')).find((r) =>
      r.textContent?.includes('Enable custom icon')
    );
    fireEvent.click(row!.querySelector('input[type="checkbox"]')!);

    expect(props.setCustomIconEnabled).toHaveBeenCalledWith(false);
    expect(props.setIconSelectedFile).toHaveBeenCalledWith('');
    await Promise.resolve();
    expect(invoke).not.toHaveBeenCalled();
  });
});
