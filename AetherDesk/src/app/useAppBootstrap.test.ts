import { renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { invokeMock, mockInvokeCommands } from '../test/tauriMocks';
import { getSettings } from '../hooks/useSettings';
import { useAppBootstrap } from './useAppBootstrap';

// Keeps the real IPC path for every other test while letting one test simulate
// a corrupt settings.json or a DPAPI failure at the settings read.
vi.mock('../hooks/useSettings', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../hooks/useSettings')>();
  return { ...actual, getSettings: vi.fn(actual.getSettings) };
});

const settings = { hubcap_api_key: 'key', steam_path: 'C:/Steam' };

const createParams = () => ({
  loadDeskVersion: vi.fn().mockResolvedValue(undefined),
  refreshAppearance: vi.fn().mockResolvedValue(undefined),
  refreshHubcapUsage: vi.fn().mockResolvedValue(undefined),
  checkDeskUpdates: vi.fn().mockResolvedValue(undefined),
  checkDllUpdates: vi.fn().mockResolvedValue(undefined),
  warnIfSteamPathMissing: vi.fn().mockResolvedValue(undefined),
});

describe('useAppBootstrap', () => {
  it('shares one settings snapshot across startup consumers', async () => {
    mockInvokeCommands({ get_settings: settings });
    const params = createParams();

    renderHook(() => useAppBootstrap(params));

    await waitFor(() => expect(params.refreshAppearance).toHaveBeenCalledWith(settings));
    expect(params.loadDeskVersion).toHaveBeenCalledOnce();
    expect(params.refreshHubcapUsage).toHaveBeenCalledWith(undefined, settings);
    expect(params.checkDeskUpdates).toHaveBeenCalledOnce();
    expect(params.checkDllUpdates).toHaveBeenCalledWith(settings);
    expect(params.warnIfSteamPathMissing).toHaveBeenCalledOnce();
    expect(invokeMock.mock.calls.filter(([command]) => command === 'get_settings')).toHaveLength(1);
    // The library scan owns the warm-up; startup must not scan the library.
    expect(invokeMock.mock.calls.filter(([command]) => command === 'warm_library_game_cache')).toHaveLength(0);
  });

  it('still hydrates the appearance and warns when the settings read fails', async () => {
    mockInvokeCommands({});
    vi.mocked(getSettings).mockRejectedValueOnce(new Error('corrupt settings.json'));
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const params = createParams();

    renderHook(() => useAppBootstrap(params));

    await waitFor(() => expect(params.warnIfSteamPathMissing).toHaveBeenCalled());
    expect(params.refreshAppearance).toHaveBeenCalledWith();
    expect(warn).toHaveBeenCalledWith('Settings load failed at startup:', expect.any(Error));
    expect(params.loadDeskVersion).toHaveBeenCalledOnce();
    await waitFor(() => expect(params.checkDeskUpdates).toHaveBeenCalledOnce());
    expect(params.refreshHubcapUsage).not.toHaveBeenCalled();
    expect(params.checkDllUpdates).not.toHaveBeenCalled();
    warn.mockRestore();
  });
});
