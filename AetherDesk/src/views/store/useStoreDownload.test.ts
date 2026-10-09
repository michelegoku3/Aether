import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { StoreGameResult } from '../../hooks/useStoreSearch';
import { invokeMock, mockInvokeCommands } from '../../test/tauriMocks';
import { useStoreDownload } from './useStoreDownload';

const { loadInstalledGames } = vi.hoisted(() => ({ loadInstalledGames: vi.fn() }));
vi.mock('../../hooks/useLibraryGames', () => ({
  useLibraryGames: () => ({ loadInstalledGames }),
}));
vi.mock('../../hooks/useModalDismiss', () => ({ useModalDismiss: vi.fn() }));

const game: StoreGameResult = {
  id: 10,
  appId: '10',
  name: 'Test Game',
  has_manifest: true,
  has_denuvo: false,
};

const configuredSettings = {
  hubcap_api_key: 'hubcap-key',
  ryuu_api_key: 'ryuu-key',
  steam_path: 'C:/Steam',
};

describe('useStoreDownload', () => {
  beforeEach(() => loadInstalledGames.mockReset());

  it('selects providers in Hubcap, LuaTools, Ryuu priority order', async () => {
    mockInvokeCommands({
      get_settings: configuredSettings,
      get_luatools_auth_status: { signedIn: true },
    });
    const { result } = renderHook(() => useStoreDownload({}));

    await act(() => result.current.openDownload(game));
    expect(result.current.source).toBe('hubcap');

    invokeMock.mockReset();
    mockInvokeCommands({
      get_settings: { ...configuredSettings, hubcap_api_key: '' },
      get_luatools_auth_status: { signedIn: true },
    });
    await act(() => result.current.openDownload(game));
    expect(result.current.source).toBe('luatools');

    invokeMock.mockReset();
    mockInvokeCommands({
      get_settings: { ...configuredSettings, hubcap_api_key: '' },
      get_luatools_auth_status: { signedIn: false },
    });
    await act(() => result.current.openDownload(game));
    expect(result.current.source).toBe('ryuu');
  });

  it('falls back to Hubcap when provider discovery fails', async () => {
    invokeMock.mockRejectedValue(new Error('offline'));
    const { result } = renderHook(() => useStoreDownload({}));

    await act(() => result.current.openDownload(game));

    expect(result.current.selectedGame).toEqual(game);
    expect(result.current.source).toBe('hubcap');
  });

  it('blocks Hubcap download when its key is missing', async () => {
    mockInvokeCommands({
      get_settings: { ...configuredSettings, hubcap_api_key: '', ryuu_api_key: '' },
      get_luatools_auth_status: { signedIn: false },
    });
    const { result } = renderHook(() => useStoreDownload({}));
    await act(() => result.current.openDownload(game));

    await act(() => result.current.downloadLatest());

    expect(result.current.status.text).toMatch(/Hubcap API Key/i);
    expect(result.current.isDownloading).toBe(false);
    expect(loadInstalledGames).not.toHaveBeenCalled();
  });

  it('downloads latest with the provider-specific command and refreshes consumers', async () => {
    vi.useFakeTimers();
    const onRefreshUsage = vi.fn().mockResolvedValue(undefined);
    mockInvokeCommands({
      get_settings: configuredSettings,
      get_luatools_auth_status: { signedIn: false },
      trigger_hubcap_download: 'Installed successfully',
    });
    const { result } = renderHook(() => useStoreDownload({ onRefreshUsage }));
    await act(() => result.current.openDownload(game));

    await act(() => result.current.downloadLatest());

    expect(invokeMock).toHaveBeenCalledWith('trigger_hubcap_download', {
      appId: 10,
      apiKey: 'hubcap-key',
    });
    expect(loadInstalledGames).toHaveBeenCalledOnce();
    expect(onRefreshUsage).toHaveBeenCalledOnce();
    expect(result.current.status).toEqual({ text: 'Installed successfully', type: 'success' });

    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(result.current.selectedGame).toBeNull();
  });

  it('uses LuaTools gameName payload and opens the version editor', async () => {
    const rows = [{
      rowId: 1,
      appId: 20,
      manifestId: '30',
      enabled: true,
      manifestInput: 'old',
    }];
    mockInvokeCommands({
      get_settings: { ...configuredSettings, hubcap_api_key: '' },
      get_luatools_auth_status: { signedIn: true },
      prepare_luatools_specific_version_download: rows,
    });
    const { result } = renderHook(() => useStoreDownload({}));
    await act(() => result.current.openDownload(game));
    expect(result.current.source).toBe('luatools');

    await act(() => result.current.downloadSpecific());

    expect(invokeMock).toHaveBeenCalledWith(
      'prepare_luatools_specific_version_download',
      { appId: 10, gameName: 'Test Game' },
    );
    expect(result.current.selectedGame).toBeNull();
    expect(result.current.versionGame).toEqual(game);
    expect(result.current.manifestRows[0].manifestInput).toBe('');
    expect(loadInstalledGames).toHaveBeenCalledOnce();
  });

  it('clears version state when the editor closes', async () => {
    mockInvokeCommands({
      get_settings: configuredSettings,
      get_luatools_auth_status: { signedIn: false },
      prepare_specific_version_download: [],
    });
    const { result } = renderHook(() => useStoreDownload({}));
    await act(() => result.current.openDownload(game));
    await act(() => result.current.downloadSpecific());
    await waitFor(() => expect(result.current.versionGame).toEqual(game));

    act(() => result.current.closeVersion());

    expect(result.current.versionGame).toBeNull();
    expect(result.current.manifestRows).toEqual([]);
  });
});
