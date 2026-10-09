import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { AppSettings } from '../../hooks/useSettings';
import { invokeMock, mockInvokeCommands } from '../../test/tauriMocks';
import { useSteamPath } from './useSteamPath';

const base: AppSettings = { hubcap_api_key: '', steam_path: 'C:/Steam' };

const createParams = (steamPath = 'C:/Steam') => ({
  steamPath,
  rawSettings: base,
  setSteamPath: vi.fn(),
  acceptPersistedSettings: vi.fn(),
  refreshCustomCss: vi.fn().mockResolvedValue(undefined),
  showStatus: vi.fn(),
});

describe('useSteamPath', () => {
  it('debounces read-only validation and reports an invalid path', async () => {
    vi.useFakeTimers();
    mockInvokeCommands({
      check_steam_path: { valid: false, normalized: 'bad', error: 'Steam.exe missing' },
    });
    const params = createParams('bad');
    const { result } = renderHook(() => useSteamPath(params));

    expect(result.current.checkStatus.state).toBe('checking');
    await act(async () => { await vi.advanceTimersByTimeAsync(400); });

    expect(invokeMock).toHaveBeenCalledWith('check_steam_path', { path: 'bad' });
    expect(result.current.checkStatus).toEqual({
      state: 'invalid',
      message: 'Steam.exe missing',
    });
  });

  it('ignores a stale validation response after the path changes', async () => {
    vi.useFakeTimers();
    let resolveOld: ((value: unknown) => void) | undefined;
    invokeMock.mockImplementation((command: string, args?: { path?: string }) => {
      if (command !== 'check_steam_path') return Promise.reject(new Error('unexpected'));
      if (args?.path === 'old') return new Promise((resolve) => { resolveOld = resolve; });
      return Promise.resolve({ valid: true, normalized: 'new', error: null });
    });
    let params = createParams('old');
    const { result, rerender } = renderHook(() => useSteamPath(params));
    await act(async () => { await vi.advanceTimersByTimeAsync(400); });

    params = { ...params, steamPath: 'new' };
    rerender();
    await act(async () => { await vi.advanceTimersByTimeAsync(400); });
    expect(result.current.checkStatus.state).toBe('idle');

    await act(async () => {
      resolveOld?.({ valid: false, normalized: 'old', error: 'stale error' });
    });
    expect(result.current.checkStatus.state).toBe('idle');
  });

  it('browse validates and persists only the normalized Steam path', async () => {
    const saved = { ...base, steam_path: 'D:/Steam' };
    mockInvokeCommands({
      pick_steam_folder: 'D:/Steam/',
      check_steam_path: { valid: true, normalized: 'D:/Steam', error: null },
      get_settings: { ...base, backend_owned: 'fresh' },
      save_settings: saved,
    });
    const params = createParams();
    const { result } = renderHook(() => useSteamPath(params));

    await act(() => result.current.browse());

    expect(invokeMock).toHaveBeenCalledWith('save_settings', {
      settings: expect.objectContaining({ steam_path: 'D:/Steam', backend_owned: 'fresh' }),
      base: expect.objectContaining({ backend_owned: 'fresh' }),
    });
    expect(params.acceptPersistedSettings).toHaveBeenCalledWith(saved);
    expect(params.setSteamPath).toHaveBeenLastCalledWith('D:/Steam');
    expect(params.refreshCustomCss).toHaveBeenCalledOnce();
  });

  it('does not persist an auto-detected invalid path', async () => {
    mockInvokeCommands({
      detect_steam_path: 'Z:/NotSteam',
      check_steam_path: { valid: false, normalized: 'Z:/NotSteam', error: 'Invalid' },
    });
    const params = createParams();
    const { result } = renderHook(() => useSteamPath(params));

    await act(() => result.current.detect());

    expect(invokeMock).not.toHaveBeenCalledWith('save_settings', expect.anything());
    expect(params.showStatus).toHaveBeenCalledWith(
      expect.stringContaining('not a valid Steam installation'),
      'error',
    );
  });
});
