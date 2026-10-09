import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { STEAM_RUNTIME_EVENT } from '../constants/library';
import {
  emitTauriEvent,
  invokeMock,
  mockInvokeCommands,
} from '../test/tauriMocks';
import { useSteamRuntime } from './useSteamRuntime';

describe('useSteamRuntime', () => {
  it('hydrates from the monitor and reacts to push events', async () => {
    mockInvokeCommands({ is_steam_running: false });
    const { result } = renderHook(() => useSteamRuntime());

    await waitFor(() => expect(result.current.running).toBe(false));
    act(() => emitTauriEvent(STEAM_RUNTIME_EVENT, true));

    expect(result.current.running).toBe(true);
  });

  it('starts Steam when it is stopped', async () => {
    invokeMock
      .mockResolvedValueOnce(false) // initial effect
      .mockResolvedValueOnce('started')
      .mockResolvedValueOnce(true);
    const { result } = renderHook(() => useSteamRuntime());
    await waitFor(() => expect(result.current.running).toBe(false));

    await act(() => result.current.runAction());

    expect(invokeMock).toHaveBeenCalledWith('start_steam');
    expect(invokeMock).not.toHaveBeenCalledWith('restart_steam');
    expect(result.current.running).toBe(true);
    expect(result.current.busy).toBe(false);
  });

  it('restarts Steam when it is already running', async () => {
    invokeMock
      .mockResolvedValueOnce(true)
      .mockResolvedValueOnce('restarted')
      .mockResolvedValueOnce(true);
    const { result } = renderHook(() => useSteamRuntime());
    await waitFor(() => expect(result.current.running).toBe(true));

    await act(() => result.current.runAction());

    expect(invokeMock).toHaveBeenCalledWith('restart_steam');
  });

  it('blocks a second action while restart is in flight', async () => {
    let finishRestart: ((value: string) => void) | undefined;
    invokeMock
      .mockResolvedValueOnce(true)
      .mockReturnValueOnce(new Promise((resolve) => { finishRestart = resolve; }))
      .mockResolvedValueOnce(true);
    const { result } = renderHook(() => useSteamRuntime());
    await waitFor(() => expect(result.current.running).toBe(true));

    act(() => { void result.current.runAction(); });
    await waitFor(() => expect(result.current.busy).toBe(true));
    act(() => { void result.current.runAction(); });

    expect(invokeMock.mock.calls.filter(([command]) => command === 'restart_steam')).toHaveLength(1);
    await act(async () => finishRestart?.('done'));
  });

  it('recovers runtime state after an action error', async () => {
    invokeMock
      .mockResolvedValueOnce(false)
      .mockRejectedValueOnce(new Error('start failed'))
      .mockResolvedValueOnce(false);
    const { result } = renderHook(() => useSteamRuntime());
    await waitFor(() => expect(result.current.running).toBe(false));

    await act(() => result.current.runAction());

    expect(result.current.running).toBe(false);
    expect(result.current.busy).toBe(false);
  });
});
