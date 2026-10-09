import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { SettingsGuard } from '../views/SettingsView';
import {
  invokeMock,
  mockInvokeCommands,
  requestWindowClose,
} from '../test/tauriMocks';
import {
  useUnsavedNavigationGuard,
  useWindowCloseGuard,
} from './useUnsavedNavigationGuard';

const validSteamCommands = {
  get_settings: { hubcap_api_key: '', steam_path: 'C:/Steam' },
  check_steam_path: { valid: true, normalized: 'C:/Steam', error: null },
};

const guard = (overrides: Partial<SettingsGuard> = {}): SettingsGuard => ({
  isDirty: () => true,
  save: vi.fn().mockResolvedValue(true),
  discard: vi.fn(),
  ...overrides,
});

const useHarness = () => {
  const navigation = useUnsavedNavigationGuard();
  useWindowCloseGuard(navigation.settingsGuardRef, navigation.beginPendingClose);
  return navigation;
};

describe('useUnsavedNavigationGuard', () => {
  it('navigates immediately when Settings are clean', () => {
    const { result } = renderHook(useUnsavedNavigationGuard);

    act(() => result.current.changeTab('store'));

    expect(result.current.activeTab).toBe('store');
    expect(result.current.showUnsavedModal).toBe(false);
  });

  it('defers leaving dirty Settings until Save succeeds', async () => {
    mockInvokeCommands(validSteamCommands);
    const settingsGuard = guard();
    const { result } = renderHook(useUnsavedNavigationGuard);
    act(() => result.current.changeTab('settings'));
    result.current.settingsGuardRef.current = settingsGuard;

    act(() => result.current.changeTab('store'));
    expect(result.current.activeTab).toBe('settings');
    expect(result.current.showUnsavedModal).toBe(true);

    await act(() => result.current.saveUnsaved());

    expect(settingsGuard.save).toHaveBeenCalledOnce();
    expect(result.current.activeTab).toBe('store');
    expect(result.current.showUnsavedModal).toBe(false);
  });

  it('keeps the modal open when Save fails', async () => {
    const settingsGuard = guard({ save: vi.fn().mockResolvedValue(false) });
    const { result } = renderHook(useUnsavedNavigationGuard);
    act(() => result.current.changeTab('settings'));
    result.current.settingsGuardRef.current = settingsGuard;
    act(() => result.current.changeTab('store'));

    await act(() => result.current.saveUnsaved());

    expect(result.current.activeTab).toBe('settings');
    expect(result.current.showUnsavedModal).toBe(true);
    expect(result.current.unsavedBusy).toBe(false);
  });

  it('discards before completing a pending tab switch', async () => {
    mockInvokeCommands(validSteamCommands);
    const settingsGuard = guard();
    const { result } = renderHook(useUnsavedNavigationGuard);
    act(() => result.current.changeTab('settings'));
    result.current.settingsGuardRef.current = settingsGuard;
    act(() => result.current.changeTab('library'));

    act(() => result.current.discardUnsaved());

    expect(settingsGuard.discard).toHaveBeenCalledOnce();
    expect(result.current.activeTab).toBe('library');
    await waitFor(() => expect(result.current.showUnsavedModal).toBe(false));
  });

  it('cancels a pending action without discarding edits', () => {
    const settingsGuard = guard();
    const { result } = renderHook(useUnsavedNavigationGuard);
    act(() => result.current.changeTab('settings'));
    result.current.settingsGuardRef.current = settingsGuard;
    act(() => result.current.changeTab('library'));

    act(() => result.current.cancelUnsaved());

    expect(settingsGuard.discard).not.toHaveBeenCalled();
    expect(result.current.activeTab).toBe('settings');
    expect(result.current.showUnsavedModal).toBe(false);
  });

  it('intercepts native close only when Settings are dirty', async () => {
    mockInvokeCommands({ ...validSteamCommands, force_close_window: undefined });
    const settingsGuard = guard();
    const { result } = renderHook(useHarness);
    result.current.settingsGuardRef.current = settingsGuard;

    let close!: ReturnType<typeof requestWindowClose>;
    act(() => { close = requestWindowClose(); });
    expect(close.preventDefault).toHaveBeenCalledOnce();
    expect(result.current.showUnsavedModal).toBe(true);

    await act(() => result.current.saveUnsaved());
    expect(invokeMock).toHaveBeenCalledWith('force_close_window');
  });

  it('lets native close proceed for clean Settings', () => {
    const { result } = renderHook(useHarness);
    result.current.settingsGuardRef.current = guard({ isDirty: () => false });

    let close!: ReturnType<typeof requestWindowClose>;
    act(() => { close = requestWindowClose(); });

    expect(close.preventDefault).not.toHaveBeenCalled();
    expect(result.current.showUnsavedModal).toBe(false);
  });

  it('a close request replaces a pending tab switch', async () => {
    mockInvokeCommands({ ...validSteamCommands, force_close_window: undefined });
    const settingsGuard = guard();
    const { result } = renderHook(useHarness);
    act(() => result.current.changeTab('settings'));
    result.current.settingsGuardRef.current = settingsGuard;
    act(() => result.current.changeTab('store'));

    act(() => { requestWindowClose(); });
    act(() => result.current.discardUnsaved());

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('force_close_window'));
    expect(result.current.activeTab).toBe('settings');
  });

  it('shows the Steam-path warning without blocking navigation', async () => {
    mockInvokeCommands({
      get_settings: { hubcap_api_key: '', steam_path: '' },
      check_steam_path: { valid: false, normalized: '', error: 'missing' },
    });
    const { result } = renderHook(useUnsavedNavigationGuard);
    act(() => result.current.changeTab('settings'));
    result.current.settingsGuardRef.current = guard({ isDirty: () => false });

    act(() => result.current.changeTab('home'));

    expect(result.current.activeTab).toBe('home');
    await waitFor(() => expect(result.current.showSteamPathWarning).toBe(true));
  });
});
