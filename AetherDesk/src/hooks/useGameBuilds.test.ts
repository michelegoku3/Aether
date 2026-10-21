import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invokeMock } from '../test/tauriMocks';
import { useGameBuilds, type BuildInfo } from './useGameBuilds';

const { useLibraryGamesMock, queryGameStateMock, invalidateGameStateMock } = vi.hoisted(() => ({
  useLibraryGamesMock: vi.fn(),
  queryGameStateMock: vi.fn(),
  invalidateGameStateMock: vi.fn(),
}));

vi.mock('./useLibraryGames', () => ({ useLibraryGames: useLibraryGamesMock }));

const APP_ID = 480;

const build: BuildInfo = { buildId: 24701871, date: '2026-01-15', title: 'Build 24701871' };

const savedBuild = {
  appId: APP_ID,
  buildId: build.buildId,
  date: build.date,
  title: build.title,
  savedAt: 1,
};

/** Renders the hook with a backend that answers the two Builds queries. */
const renderBuilds = (saved: (typeof savedBuild)[] = []) => {
  useLibraryGamesMock.mockReturnValue({
    queryGameState: queryGameStateMock,
    invalidateGameState: invalidateGameStateMock,
  });
  queryGameStateMock.mockImplementation(async (_appId: number, command: string) => {
    if (command === 'get_game_builds') return [build];
    if (command === 'get_saved_builds') return saved;
    return null;
  });
  return renderHook(() => useGameBuilds(APP_ID));
};

describe('useGameBuilds bookmarks', () => {
  beforeEach(() => {
    invalidateGameStateMock.mockReset();
  });

  it('keeps the previous bookmarks and surfaces the failure', async () => {
    invokeMock.mockRejectedValueOnce(new Error('saved_builds.json is locked'));
    const { result } = renderBuilds();
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      await expect(result.current.toggleSaved(build)).resolves.toBeUndefined();
    });

    expect(result.current.savedIds.has(build.buildId)).toBe(false);
    expect(result.current.error).toContain('locked');
  });

  it('keeps a bookmark the backend refused to remove', async () => {
    invokeMock.mockRejectedValueOnce(new Error('saved_builds.json is locked'));
    const { result } = renderBuilds([savedBuild]);
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.savedIds.has(build.buildId)).toBe(true);

    await act(async () => {
      await result.current.toggleSaved(build);
    });

    expect(result.current.savedIds.has(build.buildId)).toBe(true);
    expect(result.current.error).toContain('locked');
  });

  it('keeps the cached answer when the backend refuses', async () => {
    invokeMock.mockRejectedValueOnce(new Error('saved_builds.json is locked'));
    const { result } = renderBuilds();
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      await result.current.toggleSaved(build);
    });

    expect(invalidateGameStateMock).not.toHaveBeenCalled();
  });

  it('adds the bookmark and refreshes the cached answer', async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    const { result } = renderBuilds();
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      await result.current.toggleSaved(build);
    });

    expect(invokeMock).toHaveBeenCalledWith('save_build', {
      appId: APP_ID,
      buildId: build.buildId,
      date: build.date,
      title: build.title,
    });
    expect(result.current.savedIds.has(build.buildId)).toBe(true);
    expect(result.current.error).toBe('');
    expect(invalidateGameStateMock).toHaveBeenCalledWith(APP_ID);
  });

  it('clears the previous failure when the next toggle succeeds', async () => {
    invokeMock.mockRejectedValueOnce(new Error('saved_builds.json is locked'));
    const { result } = renderBuilds();
    await waitFor(() => expect(result.current.loading).toBe(false));
    await act(async () => {
      await result.current.toggleSaved(build);
    });
    expect(result.current.error).toContain('locked');

    invokeMock.mockResolvedValueOnce(undefined);
    await act(async () => {
      await result.current.toggleSaved(build);
    });

    expect(result.current.error).toBe('');
    expect(result.current.savedIds.has(build.buildId)).toBe(true);
  });
});
