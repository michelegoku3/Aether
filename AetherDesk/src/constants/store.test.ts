import { describe, expect, it } from 'vitest';
import { compareStoreGames, type SortableStoreGame } from './store';

const game = (name: string, overrides: Partial<SortableStoreGame> = {}): SortableStoreGame => ({
  name,
  ...overrides,
});

describe('compareStoreGames', () => {
  it('sorts newest by release_date_unix descending', () => {
    const older = game('Older', { release_date_unix: 100 });
    const newer = game('Newer', { release_date_unix: 300 });
    expect([older, newer].sort((a, b) => compareStoreGames(a, b, 'newest')).map((g) => g.name)).toEqual([
      'Newer',
      'Older',
    ]);
  });

  it('preserves order for trending and relevance', () => {
    const b = game('Beta');
    const a = game('Alpha');
    expect([b, a].sort((x, y) => compareStoreGames(x, y, 'trending')).map((g) => g.name)).toEqual(['Beta', 'Alpha']);
    expect([b, a].sort((x, y) => compareStoreGames(x, y, 'relevance')).map((g) => g.name)).toEqual(['Beta', 'Alpha']);
  });

  it('sorts alphabetically for name', () => {
    const b = game('Zeta');
    const a = game('Alpha');
    expect([b, a].sort((x, y) => compareStoreGames(x, y, 'name')).map((g) => g.name)).toEqual(['Alpha', 'Zeta']);
  });

  it('sorts biggest discount first with name tie-break', () => {
    const low = game('Alpha', { price: { discountPercent: 10, finalCents: 900 } });
    const high = game('Zeta', { price: { discountPercent: 75, finalCents: 500 } });
    const none = game('Beta', { price: null });
    const tied = game('Beta2', { price: { discountPercent: 75, finalCents: 1000 } });

    expect([low, none, high, tied].sort((a, b) => compareStoreGames(a, b, 'discount')).map((g) => g.name)).toEqual([
      'Beta2',
      'Zeta',
      'Alpha',
      'Beta',
    ]);
  });

  it('sorts price_asc with free games first and missing prices last', () => {
    const free = game('Freebie', { price: { finalCents: 0 } });
    const cheap = game('Cheap', { price: { finalCents: 499 } });
    const expensive = game('Pricey', { price: { finalCents: 5999 } });
    const unknown = game('Unknown', { price: null });

    expect(
      [unknown, expensive, cheap, free].sort((a, b) => compareStoreGames(a, b, 'price_asc')).map((g) => g.name)
    ).toEqual(['Freebie', 'Cheap', 'Pricey', 'Unknown']);
  });
});
