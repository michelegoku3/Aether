import { describe, expect, it } from 'vitest';
import { resolveLiveGame } from './library';
import type { InstalledGame } from '../hooks/useLibraryGames';

const game = (id: number, state: Partial<InstalledGame> = {}): InstalledGame => ({
  id,
  name: `Game ${id}`,
  appId: String(id),
  installDir: '',
  libraryPath: '',
  gamePath: '',
  installed: false,
  installing: false,
  ...state,
});

describe('resolveLiveGame', () => {
  it('prefers the live entry over the snapshot taken when the card was clicked', () => {
    const snapshot = game(100, { installed: false });
    const live = game(100, { installed: true, gamePath: 'C:\\Steam\\Common\\Game100' });

    expect(resolveLiveGame([live], snapshot)).toBe(live);
  });

  it('falls back to the snapshot when the game left the library', () => {
    const snapshot = game(100, { installed: true });

    expect(resolveLiveGame([], snapshot)).toBe(snapshot);
    expect(resolveLiveGame(undefined, snapshot)).toBe(snapshot);
  });

  it('keeps the selection empty when nothing was selected', () => {
    expect(resolveLiveGame([game(100)], null)).toBeNull();
  });
});
