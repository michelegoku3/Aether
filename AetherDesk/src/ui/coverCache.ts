import { isCoverAssetUrl } from './coverUrls';
import type { PersistedCoverEnvelope, ResolvedCover } from './coverTypes';

const COVER_CACHE_PREFIX = 'aether_cover_v4_';

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
 *  trims to MAX_MEMORY_ENTRIES. */
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

  clear() {
    this.map.clear();
  }
}

const memoryCoverCache = new BoundedCoverCache();

let haveSweptKeys = false;

/** One-time sweep: drop expired/malformed v4 entries so they don't waste
 *  localStorage. Lazy — runs on the first read. */
const sweepStaleEntries = () => {
  if (haveSweptKeys) return;
  haveSweptKeys = true;
  try {
    const toRemove: string[] = [];
    const kept: { key: string; cachedAt: number }[] = [];
    const now = Date.now();
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (!key) continue;
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
    // Malformed JSON (legacy plain-string or v3 payloads): return null rather
    // than guessing; the caller clears the unparseable entry.
  }
  return null;
};

export const getCachedCover = (appId: string): ResolvedCover | null => {
  sweepStaleEntries();
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

/** Drops both the in-memory LRU and the persisted entries. The Settings button
 *  owns the user-facing action; this function only forgets what this module
 *  cached, so the next cover resolution goes back to Steam. */
export const clearCachedCovers = () => {
  memoryCoverCache.clear();
  try {
    for (const key of Object.keys(localStorage)) {
      if (key.startsWith(COVER_CACHE_PREFIX)) {
        localStorage.removeItem(key);
      }
    }
  } catch {
    // localStorage unavailable: the memory cache is already gone.
  }
};
