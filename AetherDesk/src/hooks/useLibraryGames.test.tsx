import React from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { invokeMock, mockInvokeCommands } from '../test/tauriMocks';
import { LibraryGamesProvider, useLibraryGames, type InstalledGame } from './useLibraryGames';

const game = (id: number): InstalledGame => ({
  id,
  name: `Game ${id}`,
  appId: String(id),
  installDir: `Game ${id}`,
  libraryPath: 'C:/Steam',
  gamePath: `C:/Steam/steamapps/common/Game ${id}`,
  installed: true,
  installing: false,
});

const wrapper = ({ children }: { children: React.ReactNode }) =>
  React.createElement(LibraryGamesProvider, null, children);

const renderProvider = () => renderHook(() => useLibraryGames(), { wrapper });

const warmCalls = () => invokeMock.mock.calls.filter(([command]) => command === 'warm_library_game_cache');

describe('LibraryGamesProvider', () => {
  it('warms metadata once, after the scan that already enumerated the library', async () => {
    mockInvokeCommands({
      get_library_change_revision: 0,
      get_installed_library_games: [game(10), game(20)],
      warm_library_game_cache: 2,
    });

    const { result } = renderProvider();
    await waitFor(() => expect(result.current.games).toHaveLength(2));

    expect(warmCalls()).toHaveLength(1);
    expect(warmCalls()[0][1]).toEqual({ appIds: [10, 20] });

    act(() => {
      void result.current.loadInstalledGames();
    });
    await waitFor(() =>
      expect(invokeMock.mock.calls.filter(([command]) => command === 'get_installed_library_games')).toHaveLength(2)
    );

    expect(warmCalls()).toHaveLength(1);
  });

  it('does not warm after a failed scan', async () => {
    mockInvokeCommands({
      get_library_change_revision: 0,
      get_installed_library_games: () => Promise.reject(new Error('scan failed')),
    });

    const { result } = renderProvider();
    await waitFor(() => expect(result.current.isLoading).toBe(false));

    expect(result.current.status.type).toBe('error');
    expect(warmCalls()).toHaveLength(0);
  });
});
