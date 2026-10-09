import { act, renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { invokeMock, mockInvokeCommands } from '../test/tauriMocks';
import { useDllRuntime } from './useDllRuntime';
import { useHubcapUsage } from './useHubcapUsage';

describe('application runtime services', () => {
  it('loads Hubcap usage from a forced key without reading settings', async () => {
    mockInvokeCommands({ get_hubcap_usage: { usage: 12, limit: 100 } });
    const { result } = renderHook(() => useHubcapUsage());

    await act(() => result.current.refresh('forced-key'));

    expect(invokeMock).toHaveBeenCalledWith('get_hubcap_usage', { apiKey: 'forced-key' });
    expect(invokeMock).not.toHaveBeenCalledWith('get_settings');
    expect(result.current.usage).toEqual({ usage: 12, limit: 100, hasKey: true });
  });

  it('resets Hubcap usage for an empty key or backend failure', async () => {
    const { result } = renderHook(() => useHubcapUsage());
    await act(() => result.current.refresh(''));
    expect(result.current.usage).toEqual({ usage: 0, limit: 1500, hasKey: false });

    invokeMock.mockRejectedValue(new Error('quota unavailable'));
    await act(() => result.current.refresh('key'));
    expect(result.current.usage).toEqual({ usage: 0, limit: 1500, hasKey: false });
  });

  it('loads DLL install, block and version state in parallel', async () => {
    mockInvokeCommands({
      is_dll_installed: true,
      is_steam_blocked: true,
      check_aether_dll_update: { installed_version: '0.11.3' },
    });
    const { result } = renderHook(() => useDllRuntime());

    await act(() => result.current.checkStatus({ hubcap_api_key: '', steam_path: 'C:/Steam' }));

    expect(result.current.status).toEqual({
      isInstalled: true,
      installedVersion: '0.11.3',
      isSteamBlocked: true,
    });
  });

  it('skips DLL status IPC for an empty Steam path', async () => {
    const { result } = renderHook(() => useDllRuntime());

    await act(() => result.current.checkStatus({ hubcap_api_key: '', steam_path: '' }));

    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('falls back to not-installed when a DLL status request fails', async () => {
    invokeMock.mockRejectedValue(new Error('status error'));
    const { result } = renderHook(() => useDllRuntime());

    await act(() => result.current.checkStatus({ hubcap_api_key: '', steam_path: 'C:/Steam' }));

    expect(result.current.status).toEqual({
      isInstalled: false,
      installedVersion: 'N/A',
      isSteamBlocked: false,
    });
  });
});
