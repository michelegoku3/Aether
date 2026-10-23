import { act, renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { AppSettings } from '../hooks/useSettings';
import { invokeMock, mockInvokeCommands } from '../test/tauriMocks';
import { useDllRuntime } from './useDllRuntime';

const settings = { steam_path: 'C:/Steam' } as AppSettings;

describe('useDllRuntime', () => {
  it('reads the installed version locally and leaves the network check to the coordinator', async () => {
    mockInvokeCommands({
      is_dll_installed: true,
      is_steam_blocked: false,
      get_installed_dll_version: '1.2.3',
    });
    const { result } = renderHook(() => useDllRuntime());

    await act(async () => {
      await result.current.checkStatus(settings);
    });

    expect(result.current.status).toEqual({
      isInstalled: true,
      installedVersion: '1.2.3',
      isSteamBlocked: false,
    });
    expect(invokeMock.mock.calls.map(([command]) => command)).not.toContain('check_aether_dll_update');
  });

  it('falls back to N/A when the installed version is unavailable', async () => {
    mockInvokeCommands({
      is_dll_installed: false,
      is_steam_blocked: false,
      get_installed_dll_version: '',
    });
    const { result } = renderHook(() => useDllRuntime());

    await act(async () => {
      await result.current.checkStatus(settings);
    });

    expect(result.current.status.installedVersion).toBe('N/A');
  });
});
