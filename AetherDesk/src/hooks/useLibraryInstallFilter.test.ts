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

/** One game per badge, plus an "uncracked" decoy that must never match. */
const badgeGames = [
  game(1, 'Alpha', { has_denuvo: true }),
  game(2, 'Beta', { uco2_supported: true }),
  game(3, 'Gamma', { crack_status: 'cracked' }),
  game(4, 'Delta', { drm_protection: 'None' }),
  game(5, 'Epsilon', { crack_status: 'uncracked', drm_protection: 'Steam' }),
];

const renderBadgeFilter = () => renderHook(() => useLibraryInstallFilter(badgeGames, false));

describe('useLibraryInstallFilter badge filters', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('restores the persisted badge filter and persists a new one', async () => {
    localStorage.setItem('aether.library_badge_filter', 'uco2');
    const { result } = renderBadgeFilter();
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(result.current.badgeFilter).toBe('uco2');
    expect(result.current.filteredGames.map((entry) => entry.id)).toEqual([2]);

    act(() => result.current.setBadgeFilter('denuvo'));
    expect(result.current.badgeFilter).toBe('denuvo');
    expect(localStorage.getItem('aether.library_badge_filter')).toBe('denuvo');
    expect(result.current.filteredGames.map((entry) => entry.id)).toEqual([1]);
  });

  it('matches Denuvo from the flag or the DRM string', async () => {
    const { result } = renderBadgeFilter();
    await waitFor(() => expect(result.current.ready).toBe(true));

    act(() => result.current.setBadgeFilter('denuvo'));
    expect(result.current.filteredGames.map((entry) => entry.id)).toEqual([1]);

    act(() => result.current.setBadgeFilter('all'));
    expect(result.current.filteredGames).toHaveLength(badgeGames.length);
  });

  it('treats cracked and hypervisor as cracked but never uncracked', async () => {
    const { result } = renderBadgeFilter();
    await waitFor(() => expect(result.current.ready).toBe(true));

    act(() => result.current.setBadgeFilter('cracked'));
    expect(result.current.filteredGames.map((entry) => entry.id)).toEqual([3]);
  });

  it('matches DRM-Free only when the DRM string is exactly none', async () => {
    const { result } = renderBadgeFilter();
    await waitFor(() => expect(result.current.ready).toBe(true));

    act(() => result.current.setBadgeFilter('drm_free'));
    expect(result.current.filteredGames.map((entry) => entry.id)).toEqual([4]);
  });

  it('reports a badge-specific count and empty message', async () => {
    const { result } = renderBadgeFilter();
    await waitFor(() => expect(result.current.ready).toBe(true));

    act(() => result.current.setBadgeFilter('uco2'));
    expect(result.current.countLabel).toBe('1 game shown');
    expect(result.current.emptyMessage).toContain('UCO2');
  });

  it('computes badgeCounts, matches active Fix Pins only, and sorts by installed_first or issues_first', async () => {
    const issueGames = [
      game(1, 'Zeta', { installed: false }),
      game(2, 'Alpha', {
        installed: false,
        luaIssues: [{ line: 3, depotId: 10, rawValue: 'abc', problem: 'notDecimalUint64', active: true }],
      }),
      game(3, 'Beta', {
        installed: true,
        has_denuvo: true,
        luaIssues: [{ line: 5, depotId: 20, rawValue: 'xyz', problem: 'malformedCall', active: false }],
      }),
    ];

    const { result } = renderHook(() => useLibraryInstallFilter(issueGames, false));
    await waitFor(() => expect(result.current.ready).toBe(true));

    expect(result.current.badgeCounts).toEqual({
      all: 3,
      denuvo: 1,
      uco2: 0,
      cracked: 0,
      drm_free: 0,
      fix_pins: 1,
    });

    act(() => result.current.setSortMode('installed_first'));
    expect(localStorage.getItem('aether.library_sort_mode')).toBe('installed_first');
    expect(result.current.filteredGames.map((entry) => entry.id)).toEqual([3, 2, 1]);

    act(() => result.current.setSortMode('issues_first'));
    expect(result.current.filteredGames.map((entry) => entry.id)).toEqual([2, 3, 1]);

    act(() => result.current.setBadgeFilter('fix_pins'));
    expect(result.current.filteredGames.map((entry) => entry.id)).toEqual([2]);
    expect(result.current.emptyMessage).toContain('Fix Pins');
  });
});
