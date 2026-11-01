import { useState } from 'react';
import type { StoreGameResult } from '../../hooks/useStoreSearch';

export type StoreBadgeFilter = 'all' | 'denuvo' | 'uco2' | 'cracked' | 'drm_free';

// knip-ignore: unused export — consumed via useStoreBadgeFilter return value
export const STORE_BADGE_FILTERS: { id: StoreBadgeFilter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'denuvo', label: 'Denuvo' },
  { id: 'uco2', label: 'UCO2' },
  { id: 'cracked', label: 'Cracked' },
  { id: 'drm_free', label: 'DRM-Free' },
];

const LOCAL_BADGE_KEY = 'aether.store_badge_filter';

const readStoredBadgeFilter = (): StoreBadgeFilter => {
  if (typeof window === 'undefined') return 'all';
  try {
    const raw = localStorage.getItem(LOCAL_BADGE_KEY);
    return (raw as StoreBadgeFilter) ?? 'all';
  } catch {
    return 'all';
  }
};

export const matchesStoreBadgeFilter = (game: StoreGameResult, badge: StoreBadgeFilter): boolean => {
  const drm = (game.drm_protection ?? '').trim().toLowerCase();
  switch (badge) {
    case 'denuvo':
      return Boolean(game.has_denuvo) || drm.includes('denuvo');
    case 'uco2':
      return Boolean(game.uco2_supported);
    case 'cracked': {
      const status = (game.crack_status ?? '').trim().toLowerCase();
      return status === 'cracked' || status.startsWith('hv') || status.includes('hypervisor');
    }
    case 'drm_free':
      return drm === 'none';
    default:
      return true;
  }
};

export function useStoreBadgeFilter() {
  const [badgeFilter, setBadgeFilter] = useState<StoreBadgeFilter>(() => readStoredBadgeFilter());

  const setBadgeFilterPersisted = (filter: StoreBadgeFilter) => {
    setBadgeFilter(filter);
    try {
      localStorage.setItem(LOCAL_BADGE_KEY, filter);
    } catch {
      // ignore storage errors
    }
  };

  return { badgeFilter, setBadgeFilter: setBadgeFilterPersisted, STORE_BADGE_FILTERS };
}
