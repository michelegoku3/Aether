import { isCoverAssetUrl } from './coverUrls';
import type { PersistedCoverEnvelope, ResolvedCover } from './coverTypes';

const COVER_CACHE_PREFIX = 'aether_cover_v4_';
const LEGACY_COVER_CACHE_PREFIX = 'aether_cover_v3_';

/** Cover-cache policy. Entries older than TTL are transparently discarded on
 *  read so we never re-render a months-old Steam capsule that may have been
 *  replaced. The in-memory LRU is bounded to avoid unbounded growth across
 *  long sessions browsing thousands of pages, and localStorage is swept when
 *  it exceeds MAX_PERSISTED_ENTRIES. */
const COVER_CACHE_TTL_MS = 30 * 24 * 60 * 60 * 1000; // 30 days
const MAX_MEMORY_ENTRIES = 500;
const MAX_PERSISTED_ENTRIES = 200;

/** Bounded LRU map for in-memory cover resolution. `get` promotes the key to
 *  most-recent so eviction drops the least-recently-used; `set` inserts and
 *  trims to MAX_MEMORY_ENTRIES. This replaces the previous unbounded `Map`
 *  (which grew without limit across long sessions). */
class BoundedCoverCache {
  private readonly map = new Map<string, ResolvedCover>();

  get(key: string): ResolvedCover | undefined {
    const value = this.map.get(key);
    if (value === undefined) return undefined;
    this.map.delete(key);
    this.map.set(key, value);
    return value;
  }

  set(key: string, value: ResolvedCover) {
    if (this.map.has(key)) this.map.delete(key);
    this.map.set(key, value);
    while (this.map.size > MAX_MEMORY_ENTRIES) {
      const oldestKey = this.map.keys().next().value;
      if (oldestKey === undefined) break;
      this.map.delete(oldestKey);
    }
  }
}

const memoryCoverCache = new BoundedCoverCache();

let haveSweptLegacyKeys = false;

/** One-time sweep: remove v3 (no TTL, raw URL/JSON) entries so they don't
 *  waste localStorage after the bump to v4. Lazy — runs on the first read. */
const sweepLegacyAndStaleEntries = () => {
  if (haveSweptLegacyKeys) return;
  haveSweptLegacyKeys = true;
  try {
    const toRemove: string[] = [];
    const kept: { key: string; cachedAt: number }[] = [];
    const now = Date.now();
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (!key) continue;
      if (key.startsWith(LEGACY_COVER_CACHE_PREFIX)) {
        toRemove.push(key);
        continue;
      }
      if (!key.startsWith(COVER_CACHE_PREFIX)) continue;
      // Keep v4 entries whose TTL is still valid; drop anything expired.
      try {
        const raw = localStorage.getItem(key);
        if (!raw) {
          toRemove.push(key);
          continue;
        }
        const env = JSON.parse(raw) as Partial<PersistedCoverEnvelope>;
        if (
          env.v !== 4 ||
          typeof env.cachedAt !== 'number' ||
          !env.url ||
          !isCoverAssetUrl(env.url) ||
          now - env.cachedAt > COVER_CACHE_TTL_MS
        ) {
          toRemove.push(key);
          continue;
        }
        kept.push({ key, cachedAt: env.cachedAt });
      } catch {
        toRemove.push(key);
      }
    }
    toRemove.forEach((k) => {
      try {
        localStorage.removeItem(k);
      } catch {
        /* ignore */
      }
    });
    // If we're still over the persisted cap, evict the oldest entries.
    if (kept.length > MAX_PERSISTED_ENTRIES) {
      kept.sort((a, b) => a.cachedAt - b.cachedAt);
      kept.slice(MAX_PERSISTED_ENTRIES).forEach(({ key }) => {
        try {
          localStorage.removeItem(key);
        } catch {
          /* ignore */
        }
      });
    }
  } catch {
    // localStorage unavailable (private mode / quota) — nothing to do.
  }
};

const parseCachedCover = (raw: string | null): ResolvedCover | null => {
  if (!raw) return null;
  try {
    const env = JSON.parse(raw) as Partial<PersistedCoverEnvelope>;
    if (
      env.v === 4 &&
      typeof env.url === 'string' &&
      (env.fit === 'portrait' || env.fit === 'landscape') &&
      typeof env.cachedAt === 'number' &&
      Date.now() - env.cachedAt <= COVER_CACHE_TTL_MS &&
      isCoverAssetUrl(env.url)
    ) {
      return { url: env.url, fit: env.fit };
    }
  } catch {
    // Fall through: v3 entries or plain strings are rejected by the legacy
    // sweep on first read. We return null instead of guessing to stay safe.
  }
  return null;
};

export const getCachedCover = (appId: string): ResolvedCover | null => {
  sweepLegacyAndStaleEntries();
  const memoryHit = memoryCoverCache.get(appId);
  if (memoryHit) return memoryHit;

  try {
    const raw = localStorage.getItem(`${COVER_CACHE_PREFIX}${appId}`);
    const parsed = parseCachedCover(raw);
    if (parsed) {
      memoryCoverCache.set(appId, parsed);
    } else if (raw) {
      // Defensive: clear any unparseable/expired entry we just read.
      localStorage.removeItem(`${COVER_CACHE_PREFIX}${appId}`);
    }
    return parsed;
  } catch {
    return null;
  }
};

export const saveCachedCover = (appId: string, cover: ResolvedCover) => {
  memoryCoverCache.set(appId, cover);
  try {
    const env: PersistedCoverEnvelope = {
      v: 4,
      url: cover.url,
      fit: cover.fit,
      cachedAt: Date.now(),
    };
    localStorage.setItem(`${COVER_CACHE_PREFIX}${appId}`, JSON.stringify(env));
  } catch {
    // Cache is an optimization only; ignore storage quota/privacy errors.
  }
};
