import { act, renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { invokeMock, mockInvokeCommands } from '../test/tauriMocks';
import { useUpdateCoordinator } from './useUpdateCoordinator';

const settings = { hubcap_api_key: '', steam_path: 'C:/Steam' };

describe('useUpdateCoordinator', () => {
  it('loads the installed Desk version with fallback', async () => {
    mockInvokeCommands({ get_desk_version: '1.4.3' });
    const { result } = renderHook(() => useUpdateCoordinator());

    await act(() => result.current.loadDeskVersion());
    expect(result.current.deskVersion).toBe('1.4.3');

    invokeMock.mockRejectedValue(new Error('unavailable'));
    await act(() => result.current.loadDeskVersion());
    expect(result.current.deskVersion).toBe('N/A');
  });

  it('tracks stable/test update metadata', async () => {
    mockInvokeCommands({
      check_aether_desk_update: {
        update_available: true,
        is_test: true,
        installed_version: '1.4.3',
      },
      check_aether_dll_update: { update_available: true, is_test: false },
    });
    const { result } = renderHook(() => useUpdateCoordinator());

    await act(() => Promise.all([
      result.current.checkDeskUpdates(),
      result.current.checkDllUpdates(settings),
    ]));

    expect(result.current.deskUpdateAvailable).toBe(true);
    expect(result.current.deskUpdateIsTest).toBe(true);
    expect(result.current.dllUpdateAvailable).toBe(true);
    expect(result.current.dllUpdateIsTest).toBe(false);
  });

  it('skips the DLL update IPC without a configured Steam path', async () => {
    const { result } = renderHook(() => useUpdateCoordinator());

    await act(() => result.current.checkDllUpdates({ ...settings, steam_path: '' }));

    expect(invokeMock).not.toHaveBeenCalled();
    expect(result.current.dllUpdateAvailable).toBe(false);
  });

  it('manual refresh shares one settings read and checks Desk/DLL in parallel', async () => {
    mockInvokeCommands({
      get_settings: settings,
      check_aether_desk_update: { update_available: false, is_test: false },
      check_aether_dll_update: { update_available: false, is_test: false },
    });
    const { result } = renderHook(() => useUpdateCoordinator());

    await act(() => result.current.checkAllUpdates());

    expect(invokeMock).toHaveBeenCalledWith('get_settings');
    expect(invokeMock).toHaveBeenCalledWith('check_aether_desk_update');
    expect(invokeMock).toHaveBeenCalledWith('check_aether_dll_update');
  });
});
