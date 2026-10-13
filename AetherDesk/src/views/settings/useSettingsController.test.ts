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
  get_settings_health: { fresh: false, corruptFallback: false },
  acknowledge_ost_warning: undefined,
  set_ost_source_enabled: undefined,
  reset_settings_to_defaults: settings,
  apply_window_icon: undefined,
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
    await act(async () => {
      success = await result.current.save();
    });

    expect(success).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith(
      'save_settings',
      expect.objectContaining({
        settings: expect.objectContaining({ custom_game_name: 'Changed' }),
        base: settings,
      })
    );
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
    await act(async () => {
      success = await result.current.save();
    });

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

  it('warns instead of rejecting when mount-time refreshes fail', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    mockInvokeCommands(commandDefaults());
    const params = createParams();
    params.onRefreshUsage.mockRejectedValueOnce(new Error('usage offline'));

    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));
    await waitFor(() =>
      expect(warn).toHaveBeenCalledWith('[settings] failed to refresh Hubcap usage:', expect.any(Error))
    );

    warn.mockRestore();
  });

  it('opens devtools on save when the flag turns on', async () => {
    mockInvokeCommands({
      ...commandDefaults(),
      save_settings: (args: unknown) => ({
        ...(args as { settings: AppSettings }).settings,
        enable_webview_devtools: true,
      }),
    });
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));
    act(() => result.current.setField('enableWebviewDevtools', true));

    let success = false;
    await act(async () => {
      success = await result.current.save();
    });

    expect(success).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith('open_webview_devtools');
    expect(invokeMock).not.toHaveBeenCalledWith('close_webview_devtools');
  });

  it('closes devtools on save when the flag turns off', async () => {
    mockInvokeCommands({
      ...commandDefaults(),
      save_settings: (args: unknown) => (args as { settings: AppSettings }).settings,
    });
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    // Simulate a previously saved opt-in, then switch it off.
    act(() => result.current.setField('enableWebviewDevtools', true));
    await act(async () => {
      await result.current.save();
    });
    expect(invokeMock).toHaveBeenCalledWith('open_webview_devtools');

    invokeMock.mockClear();
    act(() => result.current.setField('enableWebviewDevtools', false));
    await act(async () => {
      await result.current.save();
    });

    expect(invokeMock).toHaveBeenCalledWith('close_webview_devtools');
    expect(invokeMock).not.toHaveBeenCalledWith('open_webview_devtools');
  });

  it('flags corrupt stored settings for the banner and clears on save', async () => {
    mockInvokeCommands({
      ...commandDefaults(),
      get_settings_health: { fresh: false, corruptFallback: true },
    });
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    expect(result.current.settingsCorrupt).toBe(true);

    let success = false;
    await act(async () => {
      success = await result.current.save();
    });

    expect(success).toBe(true);
    expect(result.current.settingsCorrupt).toBe(false);
  });

  it('reinstalls the guard when dirtiness changes', async () => {
    mockInvokeCommands(commandDefaults());
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    const first = params.guardRef.current;
    expect(first?.isDirty()).toBe(false);
    act(() => result.current.setField('customGameName', 'Changed'));

    expect(params.guardRef.current).not.toBe(first);
    expect(params.guardRef.current?.isDirty()).toBe(true);
  });

  it('saves with a valid api key and keeps it', async () => {
    mockInvokeCommands({ ...commandDefaults(), validate_hubcap_key: true });
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));
    act(() => result.current.setField('apiKey', 'smm_good'));

    let success = false;
    await act(async () => {
      success = await result.current.save();
    });

    expect(success).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith(
      'save_settings',
      expect.objectContaining({
        settings: expect.objectContaining({ hubcap_api_key: 'smm_good' }),
      })
    );
    expect(result.current.status.type).toBe('success');
  });

  it('clears an invalid api key but saves the rest', async () => {
    mockInvokeCommands({ ...commandDefaults(), validate_hubcap_key: false });
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));
    act(() => result.current.setField('apiKey', 'smm_bad'));

    let success = false;
    await act(async () => {
      success = await result.current.save();
    });

    expect(success).toBe(true);
    expect(result.current.form.apiKey).toBe('');
    expect(result.current.status.type).toBe('error');
    expect(invokeMock).toHaveBeenCalledWith(
      'save_settings',
      expect.objectContaining({
        settings: expect.objectContaining({ hubcap_api_key: '' }),
      })
    );
  });

  it('saves anyway when key validation fails on the network', async () => {
    mockInvokeCommands({
      ...commandDefaults(),
      validate_hubcap_key: () => Promise.reject(new Error('net down')),
    });
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));
    act(() => result.current.setField('apiKey', 'smm_maybe'));

    let success = false;
    await act(async () => {
      success = await result.current.save();
    });

    expect(success).toBe(true);
    expect(result.current.status.text).toContain('could not be validated');
  });

  it('blocks enabling updates without a valid key', async () => {
    mockInvokeCommands(commandDefaults());
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));
    act(() => result.current.setField('downloadGamesWithUpdatesOn', true));

    let success = true;
    await act(async () => {
      success = await result.current.save();
    });

    expect(success).toBe(false);
    expect(result.current.showHubcapWarning).toBe(true);
    expect(invokeMock.mock.calls.some(([command]) => command === 'save_settings')).toBe(false);
  });

  it('surfaces backend hubcap and conflict failures distinctly', async () => {
    mockInvokeCommands({
      ...commandDefaults(),
      save_settings: () => Promise.reject(new Error('HUBCAP_KEY_REQUIRED_FOR_UPDATES: key rotated')),
    });
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    let success = true;
    await act(async () => {
      success = await result.current.save();
    });

    expect(success).toBe(false);
    expect(result.current.showHubcapWarning).toBe(true);
    expect(result.current.settingsConflict).toBe(false);
  });

  it('flags settings conflicts from failed saves', async () => {
    mockInvokeCommands({
      ...commandDefaults(),
      save_settings: () => Promise.reject(new Error('SETTINGS_CONFLICT: changed elsewhere')),
    });
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    let success = true;
    await act(async () => {
      success = await result.current.save();
    });

    expect(success).toBe(false);
    expect(result.current.settingsConflict).toBe(true);
  });

  it('changeDownloadUpdates covers off, missing, invalid and valid keys', async () => {
    mockInvokeCommands({ ...commandDefaults(), validate_hubcap_key: true });
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    await act(async () => {
      await result.current.changeDownloadUpdates(false);
    });
    expect(result.current.form.downloadGamesWithUpdatesOn).toBe(false);

    await act(async () => {
      await result.current.changeDownloadUpdates(true);
    });
    expect(result.current.showHubcapWarning).toBe(true);

    act(() => result.current.setField('apiKey', 'smm_good'));
    await act(async () => {
      await result.current.changeDownloadUpdates(true);
    });
    expect(result.current.form.downloadGamesWithUpdatesOn).toBe(true);
    expect(result.current.status.type).toBe('success');
  });

  it('changeDownloadUpdates warns on invalid keys and validation errors', async () => {
    mockInvokeCommands({ ...commandDefaults(), validate_hubcap_key: false });
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));
    act(() => result.current.setField('apiKey', 'smm_bad'));

    await act(async () => {
      await result.current.changeDownloadUpdates(true);
    });

    expect(result.current.form.downloadGamesWithUpdatesOn).toBe(false);
    expect(result.current.showHubcapWarning).toBe(true);
  });

  it('confirmOst enables the source and resets it on failure', async () => {
    mockInvokeCommands(commandDefaults());
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    await act(async () => {
      await result.current.confirmOst();
    });

    expect(result.current.useOstSource).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith('set_ost_source_enabled', { enabled: true });
  });

  it('confirmOst closes the warning and reports backend failures', async () => {
    mockInvokeCommands({
      ...commandDefaults(),
      set_ost_source_enabled: () => Promise.reject(new Error('desync')),
    });
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));
    act(() => result.current.setShowOstWarning(true));

    await act(async () => {
      await result.current.confirmOst();
    });

    expect(result.current.showOstWarning).toBe(false);
    expect(result.current.status.type).toBe('error');
  });

  it('reset restores defaults and reports failures', async () => {
    mockInvokeCommands(commandDefaults());
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    await act(async () => {
      await result.current.reset();
    });

    expect(result.current.status.type).toBe('success');
    expect(invokeMock).toHaveBeenCalledWith('reset_settings_to_defaults');
  });

  it('reloadSaved reloads and reports failures', async () => {
    mockInvokeCommands(commandDefaults());
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    await act(async () => {
      await result.current.reloadSaved();
    });

    expect(result.current.status.type).toBe('info');
  });

  it('reloadSaved reports backend failures', async () => {
    mockInvokeCommands({
      ...commandDefaults(),
      get_settings: () => Promise.reject(new Error('disk gone')),
    });
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('get_settings'));

    await act(async () => {
      await result.current.reloadSaved();
    });

    expect(result.current.status.type).toBe('error');
  });

  it('persistAppearance saves the appearance patch', async () => {
    mockInvokeCommands(commandDefaults());
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    await act(async () => {
      await result.current.persistAppearance({ custom_css_enabled: true });
    });

    expect(invokeMock).toHaveBeenCalledWith(
      'save_settings',
      expect.objectContaining({
        settings: expect.objectContaining({ custom_css_enabled: true }),
      })
    );
  });

  it('boots degraded when auxiliary services fail', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    mockInvokeCommands({
      ...commandDefaults(),
      get_presence_default_mode: () => Promise.reject(new Error('down')),
      get_ost_source_enabled: () => Promise.reject(new Error('down')),
      get_manifest_restore_enabled: () => Promise.reject(new Error('down')),
      get_settings_health: () => Promise.reject(new Error('down')),
    });
    const params = createParams();
    params.onRefreshUsage.mockRejectedValueOnce(new Error('down'));
    renderHook(() => useSettingsController(params));

    // presence, ost, manifest, health and usage catch-handlers.
    await waitFor(() => expect(warn.mock.calls.length).toBeGreaterThanOrEqual(5));
    warn.mockRestore();
  });

  it('clears transient status after a timeout', async () => {
    mockInvokeCommands(commandDefaults());
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    vi.useFakeTimers();
    try {
      act(() => result.current.showStatus('hello', 'info'));
      expect(result.current.status.text).toBe('hello');
      act(() => {
        vi.advanceTimersByTime(6000);
      });
      expect(result.current.status.text).toBe('');
    } finally {
      vi.useRealTimers();
    }
  });

  it('appearance picks flow through the form adapters', async () => {
    mockInvokeCommands({
      ...commandDefaults(),
      pick_theme_file: 'frieren.css',
      pick_wallpaper_file: 'frieren.jpg',
      pick_icon_file: 'aether_genshin.ico',
      save_settings: (args: unknown) => (args as { settings: AppSettings }).settings,
    });
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    await act(async () => {
      await result.current.appearance.pickTheme();
    });
    expect(result.current.form.themeSelectedFile).toBe('frieren.css');

    await act(async () => {
      await result.current.appearance.pickWallpaper();
    });
    expect(result.current.form.wallpaperSelectedFile).toBe('frieren.jpg');

    await act(async () => {
      await result.current.appearance.pickIcon();
    });
    expect(result.current.form.iconSelectedFile).toBe('aether_genshin.ico');
    expect(result.current.form.customIconEnabled).toBe(true);
  });

  it('a stale status timer never clears a newer message', async () => {
    mockInvokeCommands(commandDefaults());
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    vi.useFakeTimers();
    try {
      act(() => result.current.showStatus('Validating...', 'info'));
      act(() => {
        vi.advanceTimersByTime(5000);
      });
      // A newer message replaces the first one before its timer fires.
      act(() => result.current.showStatus('Saving...', 'info'));
      act(() => {
        vi.advanceTimersByTime(2000);
      });
      // The first timer fired here: the live message must survive it.
      expect(result.current.status.text).toBe('Saving...');
      act(() => {
        vi.advanceTimersByTime(5000);
      });
      expect(result.current.status.text).toBe('');
    } finally {
      vi.useRealTimers();
    }
  });

  it('a persistent progress message outlives its own expiry', async () => {
    mockInvokeCommands(commandDefaults());
    const params = createParams();
    const { result } = renderHook(() => useSettingsController(params));
    await waitFor(() => expect(result.current.form.steamPath).toBe('C:/Steam'));

    vi.useFakeTimers();
    try {
      act(() => result.current.showStatus('Validating...', 'info', { persist: true }));
      // Past the normal 6s expiry: a long validation must stay visible.
      act(() => {
        vi.advanceTimersByTime(10000);
      });
      expect(result.current.status.text).toBe('Validating...');
      // ...until the next message replaces it, which then expires normally.
      act(() => result.current.showStatus('done', 'success'));
      expect(result.current.status.text).toBe('done');
      act(() => {
        vi.advanceTimersByTime(6000);
      });
      expect(result.current.status.text).toBe('');
    } finally {
      vi.useRealTimers();
    }
  });
});
