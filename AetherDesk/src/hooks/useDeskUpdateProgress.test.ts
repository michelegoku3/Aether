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

  it('keeps the latest phase and maps the download into the overall bar', async () => {
    const { result } = renderHook(() => useDeskUpdateProgress(true));
    await act(async () => {});

    act(() => {
      emitTauriEvent(DESK_UPDATE_PROGRESS_EVENT, {
        phase: 'downloading',
        downloadedBytes: 45,
        totalBytes: 180,
      });
    });

    // The download owns the 3-75 band: a quarter of the bytes is 21 % overall.
    expect(result.current).toEqual({ phase: 'downloading', progress: 21, elapsedSeconds: 0 });
  });

  it('holds the bar at the start of the download without a content length', async () => {
    const { result } = renderHook(() => useDeskUpdateProgress(true));
    await act(async () => {});

    act(() => {
      emitTauriEvent(DESK_UPDATE_PROGRESS_EVENT, {
        phase: 'downloading',
        downloadedBytes: 45,
        totalBytes: null,
      });
    });

    expect(result.current?.progress).toBe(3);
    expect(result.current?.phase).toBe('downloading');
  });

  it('anchors the other phases so the bar keeps moving past the download', async () => {
    const { result } = renderHook(() => useDeskUpdateProgress(true));
    await act(async () => {});

    const progressAt = (phase: 'looking-up' | 'extracting' | 'staging' | 'restarting') => {
      act(() => {
        emitTauriEvent(DESK_UPDATE_PROGRESS_EVENT, {
          phase,
          downloadedBytes: 0,
          totalBytes: null,
        });
      });
      return result.current?.progress;
    };

    expect(progressAt('looking-up')).toBe(3);
    expect(progressAt('extracting')).toBe(78);
    expect(progressAt('staging')).toBe(92);
    expect(progressAt('restarting')).toBe(98);
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
