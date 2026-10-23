import { beforeEach, describe, expect, it } from 'vitest';
import { clearCachedCovers, getCachedCover, saveCachedCover } from './coverCache';

const cover = {
  url: 'https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/10/library_capsule.jpg',
  fit: 'portrait' as const,
};

describe('clearCachedCovers', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('drops the memory LRU and the persisted entries', () => {
    saveCachedCover('10', cover);
    expect(getCachedCover('10')).toEqual(cover);
    expect(localStorage.getItem('aether_cover_v4_10')).toContain('library_capsule');

    clearCachedCovers();

    // A surviving memory hit would still answer here, so this also proves the
    // LRU was cleared and not just localStorage.
    expect(getCachedCover('10')).toBeNull();
    expect(localStorage.getItem('aether_cover_v4_10')).toBeNull();
  });

  it('leaves unrelated storage keys alone', () => {
    localStorage.setItem('aether_library_filter', 'installed');
    saveCachedCover('11', cover);

    clearCachedCovers();

    expect(localStorage.getItem('aether_library_filter')).toBe('installed');
  });
});
