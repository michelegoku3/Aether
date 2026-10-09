import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { invokeMock, mockInvokeCommands } from '../../test/tauriMocks';
import { useAppearanceSettings } from './useAppearanceSettings';

const assets = {
  themeExists: true,
  themeName: 'dark.css',
  wallpaperExists: true,
  wallpaperName: 'wall.png',
  iconExists: true,
  iconName: 'icon.ico',
  themesDir: 'themes',
  wallpapersDir: 'wallpapers',
  iconsDir: 'icons',
};

const createParams = () => ({
  persist: vi.fn().mockResolvedValue(undefined),
  setTheme: vi.fn(),
  setWallpaper: vi.fn(),
  setIcon: vi.fn(),
  setIconEnabled: vi.fn(),
  refreshCustomCss: vi.fn().mockResolvedValue(undefined),
  showStatus: vi.fn(),
});

describe('useAppearanceSettings', () => {
  it('loads appearance assets', async () => {
    mockInvokeCommands({ get_appearance_assets: assets });
    const params = createParams();
    const { result } = renderHook(() => useAppearanceSettings(params));

    await act(() => result.current.reload());

    expect(result.current.assets).toEqual(assets);
  });

  it('picks and immediately persists a theme', async () => {
    mockInvokeCommands({
      pick_theme_file: 'new.css',
      get_appearance_assets: assets,
    });
    const params = createParams();
    const { result } = renderHook(() => useAppearanceSettings(params));

    await act(() => result.current.pickTheme());

    expect(params.setTheme).toHaveBeenCalledWith('new.css');
    expect(params.persist).toHaveBeenCalledWith({ theme_selected_file: 'new.css' });
    expect(params.refreshCustomCss).toHaveBeenCalledOnce();
    expect(params.showStatus).toHaveBeenCalledWith('Theme selected: new.css', 'success');
    expect(result.current.picking).toBeNull();
  });

  it('treats picker cancellation as a normal no-op', async () => {
    invokeMock.mockRejectedValue(new Error('No file selected'));
    const params = createParams();
    const { result } = renderHook(() => useAppearanceSettings(params));

    await act(() => result.current.pickWallpaper());

    expect(params.persist).not.toHaveBeenCalled();
    expect(params.showStatus).not.toHaveBeenCalled();
    expect(result.current.picking).toBeNull();
  });

  it('enables and applies a newly picked icon', async () => {
    mockInvokeCommands({
      pick_icon_file: 'new.ico',
      apply_window_icon: undefined,
      get_appearance_assets: assets,
    });
    const params = createParams();
    const { result } = renderHook(() => useAppearanceSettings(params));

    await act(() => result.current.pickIcon());

    expect(params.persist).toHaveBeenCalledWith({
      icon_selected_file: 'new.ico',
      custom_icon_enabled: true,
    });
    expect(params.setIconEnabled).toHaveBeenCalledWith(true);
    expect(invokeMock).toHaveBeenCalledWith('apply_window_icon');
  });
});
