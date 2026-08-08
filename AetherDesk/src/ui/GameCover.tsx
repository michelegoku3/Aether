import { useEffect, useMemo, useState } from 'react';

// v2 intentionally ignores old URL-only cache entries created while landscape
// capsules had too much priority over proper portrait artwork.
const COVER_CACHE_PREFIX = 'aether_cover_v2_';
const MIN_USABLE_WIDTH = 120;
// Steam storesearch often returns 231x87 capsule images. They are wide, but
// perfectly usable in our landscape-cover fallback; rejecting them caused many
// valid games to show the Æ placeholder.
const MIN_USABLE_HEIGHT = 60;
const PORTRAIT_RATIO_THRESHOLD = 0.85;

const STEAM_PORTRAIT_COVER_TEMPLATES = [
  // True vertical library artwork. These must always win over canonical
  // storesearch/appdetails capsules when they exist, because capsules are
  // landscape and look tiny inside our portrait card slot.
  'https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{id}/library_600x900.jpg',
  'https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{id}/library_600x900.jpg',
  'https://shared.steamstatic.com/store_item_assets/steam/apps/{id}/library_600x900.jpg',
  'https://cdn.cloudflare.steamstatic.com/steam/apps/{id}/library_600x900.jpg',
];

const STEAM_LANDSCAPE_COVER_TEMPLATES = [
  // Wide fallbacks. They are preloaded and classified before being shown, so
  // users never see broken-image flashes while the chain is being tested.
  'https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{id}/library_header.jpg',
  'https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{id}/library_header.jpg',
  'https://shared.steamstatic.com/store_item_assets/steam/apps/{id}/library_header.jpg',
  'https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{id}/header.jpg',
  'https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{id}/header.jpg',
  'https://shared.steamstatic.com/store_item_assets/steam/apps/{id}/header.jpg',
  'https://cdn.akamai.steamstatic.com/steam/apps/{id}/header.jpg',
  'https://cdn.cloudflare.steamstatic.com/steam/apps/{id}/header.jpg',
  'https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{id}/capsule_616x353.jpg',
  'https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{id}/capsule_616x353.jpg',
  'https://shared.steamstatic.com/store_item_assets/steam/apps/{id}/capsule_616x353.jpg',
];

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

  try {
    const parsed = JSON.parse(trimmed) as Partial<ResolvedCover>;
    if (typeof parsed.url === 'string' && parsed.url.trim()) {
      return {
        url: parsed.url.trim(),
        fit: parsed.fit === 'portrait' || parsed.fit === 'landscape'
          ? parsed.fit
          : inferFitFromUrl(parsed.url),
      };
    }
  } catch {
    // Backward compatibility with the old cache format where the value was
    // just the URL string.
  }

  return {
    url: trimmed,
    fit: inferFitFromUrl(trimmed),
  };
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

const normalizeCanonicalUrl = (url?: string) => {
  const trimmed = url?.trim();
  if (!trimmed) return null;
  return trimmed.split('?')[0];
};

const buildCoverUrls = (appId: string, canonicalUrl?: string) => {
  const seen = new Set<string>();
  const urls: string[] = [];

  const push = (url: string | null) => {
    if (!url || seen.has(url)) return;
    seen.add(url);
    urls.push(url);
  };

  // Priority is deliberate:
  // 1. Try predictable portrait Steam artwork first — it fits our card slot.
  // 2. Then try canonical API/cache URLs — many new games only expose hashed
  //    landscape capsule/header URLs through appdetails/storesearch.
  // 3. Finally try predictable landscape fallbacks.
  STEAM_PORTRAIT_COVER_TEMPLATES.forEach(template => push(template.replace('{id}', appId)));
  push(normalizeCanonicalUrl(canonicalUrl));
  push(getCachedCover(appId)?.url || null);
  STEAM_LANDSCAPE_COVER_TEMPLATES.forEach(template => push(template.replace('{id}', appId)));

  return urls;
};

const classifyLoadedImage = (image: HTMLImageElement): CoverFit | null => {
  const { naturalWidth, naturalHeight } = image;

  // Skip tiny placeholder-like images instead of rendering pixelated covers.
  if (naturalWidth < MIN_USABLE_WIDTH || naturalHeight < MIN_USABLE_HEIGHT) {
    return null;
  }

  const ratio = naturalWidth / naturalHeight;
  return ratio <= PORTRAIT_RATIO_THRESHOLD ? 'portrait' : 'landscape';
};

const initialCachedCover = (appId: string) => getCachedCover(appId);

const preloadCoverChain = (
  urls: string[],
  onResolved: (cover: ResolvedCover | null) => void,
) => {
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
    [appIdString, canonicalUrl]
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
