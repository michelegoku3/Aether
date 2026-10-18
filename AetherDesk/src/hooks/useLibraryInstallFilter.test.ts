import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { useLibraryInstallFilter } from './useLibraryInstallFilter';
import type { InstalledGame } from './useLibraryGames';

const game = (id: number, name: string, state: Partial<InstalledGame> = {}): InstalledGame => ({
  id,
  name,
  appId: String(id),
  installDir: '',
  libraryPath: '',
  gamePath: '',
  installed: false,
  installing: false,
  ...state,
});

/** One game per state: exactly one is complete, one is in flight, one is off. */
const games = [game(1, 'Alpha', { installed: true }), game(2, 'Beta', { installing: true }), game(3, 'Gamma')];

const renderFilter = () => renderHook(() => useLibraryInstallFilter(games, false));

describe('useLibraryInstallFilter install states', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('cycles through All, Installed, Installing and Non-installed', async () => {
    const { result } = renderFilter();
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(result.current.filter).toBe('all');

    const expected = ['installed', 'installing', 'not_installed', 'all'];
    for (const next of expected) {
      await act(async () => {
        await result.current.cycleFilter();
      });
      expect(result.current.filter).toBe(next);
    }
  });

  it('restores a persisted Installing filter instead of falling back to All', async () => {
    localStorage.setItem('aether.library_install_filter', 'installing');
    const { result } = renderFilter();

    await waitFor(() => expect(result.current.filter).toBe('installing'));
    expect(result.current.filteredGames.map((entry) => entry.id)).toEqual([2]);
    expect(result.current.countLabel).toBe('1 installing');
  });

  it('keeps a Not-installed filter away from the installing game', async () => {
    localStorage.setItem('aether.library_install_filter', 'not_installed');
    const { result } = renderFilter();

    await waitFor(() => expect(result.current.filter).toBe('not_installed'));
    expect(result.current.filteredGames.map((entry) => entry.id)).toEqual([3]);
    expect(result.current.countLabel).toBe('1 non-installed');
  });

  it('shows only the complete game under the Installed filter', async () => {
    localStorage.setItem('aether.library_install_filter', 'installed');
    const { result } = renderFilter();

    await waitFor(() => expect(result.current.filter).toBe('installed'));
    expect(result.current.filteredGames.map((entry) => entry.id)).toEqual([1]);
  });
});
