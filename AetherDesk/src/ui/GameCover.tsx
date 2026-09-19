import { useEffect, useMemo, useState } from 'react';

// ────────────────────────────────────────────────────────────────────────────
// GameCover — resolves a capsule/cover image for a game card.
//
// Design rules:
//   • The capsule slot ONLY shows a real Steam cover/capsule asset. Hero/
//     header/background banners belong to the Modify popup, never here.
//   • An ALLOWLIST of capsule-shaped URLs is applied both when candidates are
//     built AND when cached entries are read, so stale caches (pre-fix writes
//     and evicted TTL entries alike) can never leak a hero into the slot.
//   • The in-memory cache is a bounded LRU; localStorage entries carry a TTL
//     and are swept to keep persisted data well under quota.
//   • If no cover resolves, we render the placeholder (Æ) instead of a wrong
//     hero image.
// ────────────────────────────────────────────────────────────────────────────

const COVER_CACHE_PREFIX = 'aether_cover_v4_';
const MIN_USABLE_WIDTH = 120;
const MIN_USABLE_HEIGHT = 60;
const PORTRAIT_RATIO_THRESHOLD = 0.85;

/** Cover-cache policy. Entries older than TTL are transparently discarded on
 *  read so we never re-render a months-old Steam capsule that may have been
 *  replaced. The in-memory LRU is bounded to avoid unbounded growth across
 *  long sessions browsing thousands of pages, and localStorage is swept when
 *  it exceeds MAX_PERSISTED_ENTRIES. */
const COVER_CACHE_TTL_MS = 30 * 24 * 60 * 60 * 1000; // 30 days
const MAX_MEMORY_ENTRIES = 500;
const MAX_PERSISTED_ENTRIES = 200;
const LEGACY_COVER_CACHE_PREFIX = 'aether_cover_v3_';

type CoverFit = 'portrait' | 'landscape';

interface ResolvedCover {
  url: string;
  fit: CoverFit;
}

interface PersistedCoverEnvelope {
  v: 4;
  url: string;
  fit: CoverFit;
  /** Epoch ms at which this entry was written. */
  cachedAt: number;
}

interface GameCoverProps {
  appId: string | number;
  name: string;
  canonicalUrl?: string;
}

// Predictable CDN capsule paths for older apps whose covers are still served
// from the un-hashed URL scheme. Modern apps need the hashed URL provided via
// `canonicalUrl` (storesearch/appdetails), which is tried first.
const STEAM_CAPSULE_FALLBACK_TEMPLATES = [
  'https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{id}/capsule_231x87.jpg',
  'https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{id}/capsule_231x87.jpg',
  'https://shared.steamstatic.com/store_item_assets/steam/apps/{id}/capsule_231x87.jpg',
  'https://cdn.akamai.steamstatic.com/steam/apps/{id}/capsule_231x87.jpg',
  'https://cdn.cloudflare.steamstatic.com/steam/apps/{id}/capsule_231x87.jpg',
  'https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{id}/capsule_616x353.jpg',
  'https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{id}/capsule_616x353.jpg',
  'https://shared.steamstatic.com/store_item_assets/steam/apps/{id}/capsule_616x353.jpg',
];

// The ONLY URL shapes that count as a cover/capsule. Anything not matching
// (library_hero, header, background, hero_capsule, .../bg.jpg, etc.) is
// rejected so it can never land in the capsule slot.
const isCoverAssetUrl = (url: string): boolean => {
  const lower = url.toLowerCase();
  // `capsule_` alone matches `hero_capsule` (a landscape banner) — exclude.
  if (lower.includes('hero_capsule') || lower.includes('library_hero') || lower.includes('/header')) {
    return false;
  }
  return lower.includes('library_capsule')
    || lower.includes('main_capsule')
    || lower.includes('small_capsule')
    || lower.includes('capsule_231x87')
    || lower.includes('capsule_616x353')
    || lower.includes('library_600x900');
};

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
const inFlightCoverLookups = new Set<string>();

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
        if (!raw) { toRemove.push(key); continue; }
        const env = JSON.parse(raw) as Partial<PersistedCoverEnvelope>;
        if (env.v !== 4 || typeof env.cachedAt !== 'number' || !env.url
            || !isCoverAssetUrl(env.url)
            || now - env.cachedAt > COVER_CACHE_TTL_MS) {
          toRemove.push(key);
          continue;
        }
        kept.push({ key, cachedAt: env.cachedAt });
      } catch {
        toRemove.push(key);
      }
    }
    toRemove.forEach((k) => { try { localStorage.removeItem(k); } catch { /* ignore */ } });
    // If we're still over the persisted cap, evict the oldest entries.
    if (kept.length > MAX_PERSISTED_ENTRIES) {
      kept.sort((a, b) => a.cachedAt - b.cachedAt);
      kept.slice(MAX_PERSISTED_ENTRIES).forEach(({ key }) => {
        try { localStorage.removeItem(key); } catch { /* ignore */ }
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
    if (env.v === 4
        && typeof env.url === 'string'
        && (env.fit === 'portrait' || env.fit === 'landscape')
        && typeof env.cachedAt === 'number'
        && Date.now() - env.cachedAt <= COVER_CACHE_TTL_MS
        && isCoverAssetUrl(env.url)) {
      return { url: env.url, fit: env.fit };
    }
  } catch {
    // Fall through: v3 entries or plain strings are rejected by the legacy
    // sweep on first read. We return null instead of guessing to stay safe.
  }
  return null;
};

const getCachedCover = (appId: string): ResolvedCover | null => {
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

const saveCachedCover = (appId: string, cover: ResolvedCover) => {
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

const normalizeCanonicalUrl = (url?: string): string | null => {
  const trimmed = url?.trim();
  if (!trimmed) return null;
  return trimmed.split('?')[0];
};

// Build the ordered list of cover candidates. Only cover-shaped URLs survive;
// hero/header/background URLs are dropped here, before any fetch happens.
const buildCoverUrls = (appId: string, canonicalUrl?: string): string[] => {
  const seen = new Set<string>();
  const urls: string[] = [];

  const push = (url: string | null) => {
    if (!url || seen.has(url)) return;
    if (!isCoverAssetUrl(url)) return;
    seen.add(url);
    urls.push(url);
  };

  // Priority: canonical API/store URL, then resolved cover cache, then
  // predictable capsule CDN paths. No header/hero fallback, ever.
  push(normalizeCanonicalUrl(canonicalUrl));
  push(getCachedCover(appId)?.url || null);
  STEAM_CAPSULE_FALLBACK_TEMPLATES.forEach((template) => push(template.replace('{id}', appId)));

  return urls;
};

const classifyLoadedImage = (image: HTMLImageElement): CoverFit | null => {
  const { naturalWidth, naturalHeight } = image;
  if (naturalWidth < MIN_USABLE_WIDTH || naturalHeight < MIN_USABLE_HEIGHT) {
    return null;
  }
  const ratio = naturalWidth / naturalHeight;
  return ratio <= PORTRAIT_RATIO_THRESHOLD ? 'portrait' : 'landscape';
};

const initialCachedCover = (appId: string): ResolvedCover | null => getCachedCover(appId);

const preloadCoverChain = (
  urls: string[],
  onResolved: (cover: ResolvedCover | null) => void,
): (() => void) => {
  let cancelled = false;
  let index = 0;

  const tryNext = () => {
    if (cancelled) return;
    const url = urls[index];
    if (!url) {
      onResolved(null);
      return;
    }
    const image = new Image();
    image.decoding = 'async';
    image.onload = () => {
      if (cancelled) return;
      const fit = classifyLoadedImage(image);
      if (!fit) {
        index += 1;
        tryNext();
        return;
      }
      onResolved({ url, fit });
    };
    image.onerror = () => {
      index += 1;
      tryNext();
    };
    image.src = url;
  };

  tryNext();

  return () => {
    cancelled = true;
  };
};

export interface GameCoverPreloadInput {
  appId: string | number;
  imageUrl?: string;
}

export const preloadGameCovers = (games: GameCoverPreloadInput[], maxCount = 40) => {
  games.slice(0, maxCount).forEach((game) => {
    const appIdString = String(game.appId);
    if (getCachedCover(appIdString) || inFlightCoverLookups.has(appIdString)) {
      return;
    }
    inFlightCoverLookups.add(appIdString);
    preloadCoverChain(buildCoverUrls(appIdString, game.imageUrl), (cover) => {
      inFlightCoverLookups.delete(appIdString);
      if (cover) {
        saveCachedCover(appIdString, cover);
      }
    });
  });
};

export const GameCover = ({ appId, name, canonicalUrl }: GameCoverProps) => {
  const appIdString = String(appId);
  const urls = useMemo(
    () => buildCoverUrls(appIdString, canonicalUrl),
    [appIdString, canonicalUrl],
  );
  const [resolvedCover, setResolvedCover] = useState<ResolvedCover | null>(() => initialCachedCover(appIdString));
  const [hasFinishedLookup, setHasFinishedLookup] = useState(() => Boolean(initialCachedCover(appIdString)));

  useEffect(() => {
    const cachedCover = getCachedCover(appIdString);
    setResolvedCover(cachedCover);
    setHasFinishedLookup(Boolean(cachedCover));

    return preloadCoverChain(urls, (cover) => {
      setResolvedCover(cover);
      setHasFinishedLookup(true);
      if (cover) {
        saveCachedCover(appIdString, cover);
      }
    });
  }, [appIdString, urls]);

  return (
    <div className={`game-cover-wrapper ${resolvedCover?.fit === 'landscape' ? 'landscape' : ''}`}>
      {resolvedCover?.fit === 'landscape' ? (
        <img src={resolvedCover.url} alt="" className="game-cover-backdrop" aria-hidden="true" />
      ) : null}

      {resolvedCover ? (
        <img
          src={resolvedCover.url}
          alt={name}
          className={`game-cover-image ${resolvedCover.fit}`}
          loading="eager"
        />
      ) : null}

      {!resolvedCover ? (
        <div className={`game-cover-fallback ${hasFinishedLookup ? 'not-found' : 'loading'}`}>
          <span>Æ</span>
        </div>
      ) : null}
    </div>
  );
};
