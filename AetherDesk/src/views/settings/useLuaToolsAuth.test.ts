import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { invokeMock, mockInvokeCommands } from '../../test/tauriMocks';
import { useLuaToolsAuth } from './useLuaToolsAuth';

const signedIn = { signedIn: true, displayName: 'Luie', email: 'luie@example.test' };

describe('useLuaToolsAuth', () => {
  it('loads the current authentication status', async () => {
    mockInvokeCommands({ get_luatools_auth_status: signedIn });
    const { result } = renderHook(() => useLuaToolsAuth(vi.fn()));

    await act(() => result.current.refresh());

    expect(result.current.auth).toEqual(signedIn);
  });

  it('opens and resets the login modal', () => {
    const { result } = renderHook(() => useLuaToolsAuth(vi.fn()));

    act(() => result.current.open());
    act(() => result.current.changeMode('code'));
    act(() => result.current.changeCode('a-b c123'));

    expect(result.current.modalOpen).toBe(true);
    expect(result.current.mode).toBe('code');
    expect(result.current.code).toBe('ABC123');

    act(() => result.current.close());
    expect(result.current.modalOpen).toBe(false);
    expect(result.current.mode).toBe('choice');
    expect(result.current.code).toBe('');
  });

  it('rejects an invalid private code without IPC', async () => {
    const { result } = renderHook(() => useLuaToolsAuth(vi.fn()));
    act(() => result.current.changeCode('123'));

    await act(() => result.current.codeSignIn());

    expect(result.current.error).toMatch(/6-character code/i);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('completes private-code sign in and closes the modal', async () => {
    mockInvokeCommands({ sign_in_luatools_with_code: signedIn });
    const showStatus = vi.fn();
    const { result } = renderHook(() => useLuaToolsAuth(showStatus));
    act(() => result.current.open());
    act(() => result.current.changeMode('code'));
    act(() => result.current.changeCode('abc123'));

    await act(() => result.current.codeSignIn());

    expect(invokeMock).toHaveBeenCalledWith('sign_in_luatools_with_code', { code: 'ABC123' });
    expect(result.current.auth).toEqual(signedIn);
    expect(result.current.modalOpen).toBe(false);
    expect(showStatus).toHaveBeenCalledWith('LuaTools connected privately as Luie.', 'success');
  });

  it('supports OAuth cancellation and clears busy state', async () => {
    let resolveSignIn: ((value: typeof signedIn) => void) | undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === 'sign_in_luatools') {
        return new Promise<typeof signedIn>((resolve) => { resolveSignIn = resolve; });
      }
      if (command === 'cancel_luatools_sign_in') return Promise.resolve();
      return Promise.reject(new Error(`Unexpected command ${command}`));
    });
    const showStatus = vi.fn();
    const { result } = renderHook(() => useLuaToolsAuth(showStatus));

    act(() => { void result.current.signIn(); });
    await waitFor(() => expect(result.current.oauthBusy).toBe(true));
    await act(() => result.current.cancelOAuth());

    expect(result.current.oauthBusy).toBe(false);
    expect(result.current.busy).toBe(false);
    expect(showStatus).toHaveBeenCalledWith('LuaTools sign-in cancelled.', 'info');
    await act(async () => resolveSignIn?.(signedIn));
  });

  it('signs out and resets the local identity', async () => {
    mockInvokeCommands({
      get_luatools_auth_status: signedIn,
      sign_out_luatools: undefined,
    });
    const showStatus = vi.fn();
    const { result } = renderHook(() => useLuaToolsAuth(showStatus));
    await act(() => result.current.refresh());

    await act(() => result.current.signOut());

    expect(result.current.auth.signedIn).toBe(false);
    expect(showStatus).toHaveBeenCalledWith('LuaTools disconnected.', 'success');
  });
});
