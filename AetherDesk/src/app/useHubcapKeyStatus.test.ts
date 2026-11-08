import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { HUBCAP_KEY_STATUS_EVENT } from '../constants/providers';
import { emitTauriEvent, listenMock } from '../test/tauriMocks';
import { useHubcapKeyStatus } from './useHubcapKeyStatus';
import type { HubcapUsage } from './types';

const baseUsage: HubcapUsage = { usage: 0, limit: 25, hasKey: true, active: null };

describe('useHubcapKeyStatus', () => {
  it('reacts to the backend verdict and lets the user dismiss it', async () => {
    const { result } = renderHook(() => useHubcapKeyStatus(baseUsage));
    await waitFor(() => expect(listenMock).toHaveBeenCalledWith(HUBCAP_KEY_STATUS_EVENT, expect.any(Function)));

    act(() => emitTauriEvent(HUBCAP_KEY_STATUS_EVENT, { state: 'inactive', reason: 'unauthorized' }));

    expect(result.current.inactive).toBe(true);
    expect(result.current.reason).toBe('unauthorized');
    expect(result.current.dismissed).toBe(false);

    act(() => result.current.dismiss());
    expect(result.current.dismissed).toBe(true);
  });

  it('uses the startup usage answer when the push event was missed', () => {
    const { result, rerender } = renderHook(({ usage }) => useHubcapKeyStatus(usage), {
      initialProps: { usage: baseUsage },
    });

    rerender({ usage: { ...baseUsage, active: false, reason: 'account_inactive' } });

    expect(result.current.inactive).toBe(true);
    expect(result.current.reason).toBe('account_inactive');
  });

  it('ignores a verdict that belongs to the previous key', async () => {
    const { result } = renderHook(({ usage }) => useHubcapKeyStatus(usage), {
      initialProps: {
        usage: { ...baseUsage, keyFingerprint: 'aaaaaaaaaaaaaaaa', active: true },
      },
    });
    await waitFor(() => expect(listenMock).toHaveBeenCalled());

    act(() =>
      emitTauriEvent(HUBCAP_KEY_STATUS_EVENT, {
        state: 'inactive',
        reason: 'unauthorized',
        fingerprint: 'bbbbbbbbbbbbbbbb',
      })
    );

    // A request still in flight for the replaced key cannot flag the new one.
    expect(result.current.inactive).toBe(false);
    expect(result.current.dismissed).toBe(false);
  });

  it('accepts a verdict for the key it is showing', async () => {
    const { result } = renderHook(() =>
      useHubcapKeyStatus({ ...baseUsage, keyFingerprint: 'aaaaaaaaaaaaaaaa', active: true })
    );
    await waitFor(() => expect(listenMock).toHaveBeenCalled());

    act(() =>
      emitTauriEvent(HUBCAP_KEY_STATUS_EVENT, {
        state: 'inactive',
        reason: 'account_inactive',
        fingerprint: 'aaaaaaaaaaaaaaaa',
      })
    );

    expect(result.current.inactive).toBe(true);
    expect(result.current.reason).toBe('account_inactive');
  });

  it('keeps a quiet answer out of the global warning', () => {
    const { result } = renderHook(() =>
      useHubcapKeyStatus({ ...baseUsage, active: false, reason: 'unauthorized', quiet: true })
    );

    // The badge reads `active`; the modal must not, because the Settings save
    // already showed the inline message.
    expect(result.current.inactive).toBe(false);
    expect(result.current.state).toBe('unknown');
  });

  it('clears a previous dismissal when a valid key is saved', () => {
    const { result, rerender } = renderHook(({ usage }) => useHubcapKeyStatus(usage), {
      initialProps: { usage: { ...baseUsage, active: false } },
    });
    act(() => result.current.dismiss());
    expect(result.current.dismissed).toBe(true);

    rerender({ usage: { ...baseUsage, active: true } });

    expect(result.current.inactive).toBe(false);
    expect(result.current.dismissed).toBe(false);
  });
});
