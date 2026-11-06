import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { emitTauriEvent, listenMock, resetTauriMocks } from '../test/tauriMocks';
import { DESK_UPDATE_PROGRESS_EVENT } from '../constants/updates';
import { useDeskUpdateProgress } from './useDeskUpdateProgress';

describe('useDeskUpdateProgress', () => {
  beforeEach(() => {
    resetTauriMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('keeps the latest phase and computes the percentage', async () => {
    const { result } = renderHook(() => useDeskUpdateProgress(true));
    await act(async () => {});

    act(() => {
      emitTauriEvent(DESK_UPDATE_PROGRESS_EVENT, {
        phase: 'downloading',
        downloadedBytes: 45,
        totalBytes: 180,
      });
    });

    expect(result.current).toEqual({ phase: 'downloading', percent: 25, elapsedSeconds: 0 });
  });

  it('reports no percentage when the server sends no content length', async () => {
    const { result } = renderHook(() => useDeskUpdateProgress(true));
    await act(async () => {});

    act(() => {
      emitTauriEvent(DESK_UPDATE_PROGRESS_EVENT, {
        phase: 'downloading',
        downloadedBytes: 45,
        totalBytes: null,
      });
    });

    expect(result.current?.percent).toBeNull();
    expect(result.current?.phase).toBe('downloading');
  });

  it('counts elapsed seconds while the update runs', async () => {
    const { result } = renderHook(() => useDeskUpdateProgress(true));
    await act(async () => {});

    act(() => {
      emitTauriEvent(DESK_UPDATE_PROGRESS_EVENT, {
        phase: 'downloading',
        downloadedBytes: 1,
        totalBytes: 10,
      });
    });
    act(() => {
      vi.advanceTimersByTime(12_000);
    });

    expect(result.current?.elapsedSeconds).toBe(12);
  });

  it('unsubscribes when the hook goes inactive', async () => {
    let resolveListen: ((off: () => void) => void) | undefined;
    listenMock.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveListen = resolve;
        })
    );
    const off = vi.fn();
    const { unmount } = renderHook(() => useDeskUpdateProgress(true));

    unmount();
    await act(async () => resolveListen?.(off));

    expect(off).toHaveBeenCalledTimes(1);
  });
});
