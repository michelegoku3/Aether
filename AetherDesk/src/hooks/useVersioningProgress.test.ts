import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { listenMock } from '../test/tauriMocks';
import { useVersioningProgress } from './useVersioningProgress';

describe('useVersioningProgress', () => {
  it('unsubscribes when listen resolves after cleanup', async () => {
    let resolveListen: ((off: () => void) => void) | undefined;
    listenMock.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveListen = resolve;
        })
    );
    const off = vi.fn();
    const { unmount } = renderHook(() => useVersioningProgress(480, true));

    unmount();
    await act(async () => resolveListen?.(off));

    expect(off).toHaveBeenCalledTimes(1);
  });
});
