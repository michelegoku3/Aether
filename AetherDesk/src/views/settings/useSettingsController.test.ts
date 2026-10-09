import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { AppSettings } from '../../hooks/useSettings';
import { invokeMock, mockInvokeCommands } from '../../test/tauriMocks';
import type { SettingsGuard } from './types';
import { useSettingsController } from './useSettingsController';

const settings: AppSettings = {
  hubcap_api_key: '',
  steam_path: 'C:/Steam',
  store_currency: 'eur',
  custom_game_name: 'Original',
};

const commandDefaults = (saved: AppSettings = settings) => ({
  get_settings: settings,
  get_appearance_assets: {
    themeExists: false,
    themeName: null,
    wallpaperExists: false,
    wallpaperName: null,
    iconExists: false,
    iconName: null,
    themesDir: '',
    wallpapersDir: '',
    iconsDir: '',
  },
  get_presence_default_mode: true,
  get_ost_source_enabled: false,
  get_manifest_restore_enabled: true,
  get_luatools_auth_status: { signedIn: false },
  check_steam_path: { valid: true, normalized: 'C:/Steam', error: null },
  save_settings: saved,
});

const createParams = () => ({
  onRefreshUsage: vi.fn().mockResolvedValue(undefined),
  onRefreshCustomCss: vi.fn().mockResolvedValue(undefined),
  onCustomCssChange: vi.fn(),
  onPreviewPersonalWallpaper: vi.fn(),
  onPreviewAlternativeCards: vi.fn(),
  onMissingSteamPath: vi.fn(),
  guardRef: { current: null as SettingsGuard | null },
});

describe('useSettingsController', () => {
  it('hydrates settings and installs the unsaved-settings guard', async () => {
    mockInvokeCommands(commandDefaults());
    const params = createParams();
    const { result, unmount } = renderHook(() => useSettingsController(params));

    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));
    expect(result.current.form.customGameName).toBe('Original');
    expect(result.current.presenceDefaultShowOnline).toBe(true);
    expect(params.guardRef.current?.isDirty()).toBe(false);

    act(() => result.current.setField('customGameName', 'Changed'));
    expect(params.guardRef.current?.isDirty()).toBe(true);

    unmount();
    expect(params.guardRef.current).toBeNull();
  });

  it('saves through the guard and refreshes dependent runtime state', async () => {
    const saved = { ...settings, custom_game_name: 'Changed' };
    mockInvokeCommands(commandDefaults(saved));
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));
    act(() => result.current.setField('customGameName', 'Changed'));

    let success = false;
    await act(async () => { success = await result.current.save(); });

    expect(success).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith('save_settings', expect.objectContaining({
      settings: expect.objectContaining({ custom_game_name: 'Changed' }),
      base: settings,
    }));
    expect(params.onRefreshUsage).toHaveBeenLastCalledWith('');
    expect(params.onRefreshCustomCss).toHaveBeenCalled();
    expect(result.current.status.type).toBe('success');
    expect(params.guardRef.current?.isDirty()).toBe(false);
  });

  it('blocks save and signals the host when Steam path validation fails', async () => {
    mockInvokeCommands({
      ...commandDefaults(),
      check_steam_path: { valid: false, normalized: '', error: 'missing' },
    });
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    let success = true;
    await act(async () => { success = await result.current.save(); });

    expect(success).toBe(false);
    expect(params.onMissingSteamPath).toHaveBeenCalledOnce();
    expect(invokeMock.mock.calls.some(([command]) => command === 'save_settings')).toBe(false);
  });

  it('discard restores baseline values and runtime previews', async () => {
    mockInvokeCommands(commandDefaults());
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));
    act(() => result.current.setField('customGameName', 'Changed'));

    act(() => params.guardRef.current?.discard());

    expect(result.current.form.customGameName).toBe('Original');
    expect(params.onCustomCssChange).toHaveBeenCalledWith(false);
    expect(params.onPreviewPersonalWallpaper).toHaveBeenCalledWith(false, 20);
    expect(params.onPreviewAlternativeCards).toHaveBeenCalledWith(100, 50);
  });
});
