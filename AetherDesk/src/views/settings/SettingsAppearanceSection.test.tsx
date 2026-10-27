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

  it('labels the pickers in sentence case and places the kit under the theme', () => {
    render(<SettingsAppearanceSection {...baseProps()} />);

    const theme = screen.getByRole('button', { name: 'Theme' });
    const kit = screen.getByRole('button', { name: 'Kit' });
    expect(screen.getByRole('button', { name: 'Wallpaper' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Icon' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'THEME' })).not.toBeInTheDocument();
    expect(theme.compareDocumentPosition(kit) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
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
