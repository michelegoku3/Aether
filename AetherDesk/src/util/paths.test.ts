import { describe, expect, it } from 'vitest';
import { folderOf, pathFromGameRoot, shortenPath } from './paths';

describe('shortenPath', () => {
  it('returns short paths unchanged', () => {
    expect(shortenPath('C:\\Games\\Foo\\game.exe')).toBe('C:\\Games\\Foo\\game.exe');
  });

  it('truncates ASCII paths within budget', () => {
    const long = `C:\\${'a'.repeat(100)}\\game.exe`;
    const out = shortenPath(long, 80);
    expect(out.endsWith('...')).toBe(true);
    expect(Array.from(out).length).toBeLessThanOrEqual(80);
  });

  it('never splits emoji into lone surrogates', () => {
    const long = `C:\\Games\\${'🎮'.repeat(100)}\\game.exe`;
    const out = shortenPath(long, 80);
    expect(out.endsWith('...')).toBe(true);
    expect(Array.from(out).length).toBeLessThanOrEqual(80);
    // No lone surrogates: every UTF-16 unit belongs to a valid pair.
    // eslint-disable-next-line no-misleading-character-class
    expect(out).not.toMatch(/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/);
  });
});

describe('folderOf', () => {
  it('returns the parent folder for both separators', () => {
    expect(folderOf('C:\\Games\\Foo\\game.exe')).toBe('C:\\Games\\Foo');
    expect(folderOf('/home/user/game')).toBe('/home/user');
  });
});

describe('pathFromGameRoot', () => {
  it('shows paths relative to the common folder', () => {
    expect(pathFromGameRoot('C:\\Steam\\steamapps\\common\\Foo', 'C:\\Steam\\steamapps\\common\\Foo\\game.exe')).toBe(
      'Foo\\game.exe'
    );
  });

  it('falls back to the absolute path outside the game root', () => {
    expect(pathFromGameRoot('C:\\Steam\\steamapps\\common\\Foo', 'D:\\Other\\game.exe')).toBe('D:\\Other\\game.exe');
  });
});
