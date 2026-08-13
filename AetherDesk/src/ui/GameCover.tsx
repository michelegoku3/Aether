import { useEffect, useMemo, useState } from 'react';

// ────────────────────────────────────────────────────────────────────────────
// GameCover — resolves a capsule/cover image for a game card.
//
// Design rules (rewritten cleanly, no incremental patches):
//   • The capsule slot must ONLY ever show a real Steam cover/capsule asset.
//     Hero/header/background banners belong to the Modify popup, never here.
//   • Instead of trying to blacklist every "hero" URL shape (that kept leaking
//     cases like .../bg.jpg from appdetails, or landscape capsules that look
//     like heroes), we use an ALLOWLIST: an image is accepted only if its URL
//     is recognizably a capsule/cover asset. Everything else is skipped.
//   • The allowlist is applied when the URL list is built AND when a cached
//     entry is read, so a stale cache (e.g. written before this fix) can never
//     put a hero in the capsule slot.
//   • If no cover resolves, we render the placeholder (Æ) instead of a wrong
//     hero image.
// ────────────────────────────────────────────────────────────────────────────

const COVER_CACHE_PREFIX = 'aether_cover_v3_';
const MIN_USABLE_WIDTH = 120;
const MIN_USABLE_HEIGHT = 60;
const PORTRAIT_RATIO_THRESHOLD = 0.85;

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
// this allowlist (library_hero, header, background, hero_capsule, .../bg.jpg,
// etc.) is rejected so it can never land in the capsule slot.
const isCoverAssetUrl = (url: string): boolean => {
  const lower = url.toLowerCase();
  // `capsule_` alone matches `hero_capsule` (a landscape banner). Exclude it.
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

type CoverFit = 'portrait' | 'landscape';

interface ResolvedCover {
  url: string;
  fit: CoverFit;
}

interface GameCoverProps {
  appId: string | number;
  name: string;
  canonicalUrl?: string;
}

const memoryCoverCache = new Map<string, ResolvedCover>();
const inFlightCoverLookups = new Set<string>();

const inferFitFromUrl = (url: string): CoverFit => {
  const normalized = url.toLowerCase();
  return normalized.includes('library_600x900') ? 'portrait' : 'landscape';
};

const parseCachedCover = (raw: string): ResolvedCover | null => {
  const trimmed = raw.trim();
  if (!trimmed) return null;

  let url = trimmed;
  let fit: CoverFit | undefined;
  try {
    const parsed = JSON.parse(trimmed) as Partial<ResolvedCover>;
    if (typeof parsed.url === 'string' && parsed.url.trim()) {
      url = parsed.url.trim();
      fit = parsed.fit === 'portrait' || parsed.fit === 'landscape' ? parsed.fit : undefined;
    }
  } catch {
    // Old format: the value was just the URL string.
  }

  // Reject non-cover URLs on read so a stale cache can never leak a hero.
  if (!isCoverAssetUrl(url)) return null;
  return { url, fit: fit ?? inferFitFromUrl(url) };
};

const getCachedCover = (appId: string): ResolvedCover | null => {
  const memoryHit = memoryCoverCache.get(appId);
  if (memoryHit) return memoryHit;

  try {
    const raw = localStorage.getItem(`${COVER_CACHE_PREFIX}${appId}`);
    const parsed = raw ? parseCachedCover(raw) : null;
    if (parsed) {
      memoryCoverCache.set(appId, parsed);
    }
    return parsed;
  } catch {
    return null;
  }
};

const saveCachedCover = (appId: string, cover: ResolvedCover) => {
  memoryCoverCache.set(appId, cover);
  try {
    localStorage.setItem(`${COVER_CACHE_PREFIX}${appId}`, JSON.stringify(cover));
  } catch {
    // Cache is an optimization only. Ignore storage quota/privacy errors.
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
