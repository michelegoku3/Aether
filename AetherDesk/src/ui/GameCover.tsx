import { useEffect, useMemo, useState } from 'react';

const COVER_CACHE_PREFIX = 'aether_cover_';
const MIN_USABLE_WIDTH = 120;
const MIN_USABLE_HEIGHT = 90;
const PORTRAIT_RATIO_THRESHOLD = 0.85;

const STEAM_COVER_TEMPLATES = [
  // Prefer true vertical library artwork first. These match the card ratio best.
  'https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{id}/library_600x900.jpg',
  'https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{id}/library_600x900.jpg',
  'https://shared.steamstatic.com/store_item_assets/steam/apps/{id}/library_600x900.jpg',
  'https://cdn.cloudflare.steamstatic.com/steam/apps/{id}/library_600x900.jpg',

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

const getCachedCover = (appId: string) => {
  try {
    return localStorage.getItem(`${COVER_CACHE_PREFIX}${appId}`) || null;
  } catch {
    return null;
  }
};

const saveCachedCover = (appId: string, url: string) => {
  try {
    localStorage.setItem(`${COVER_CACHE_PREFIX}${appId}`, url);
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

  push(getCachedCover(appId));
  push(normalizeCanonicalUrl(canonicalUrl));
  STEAM_COVER_TEMPLATES.forEach(template => push(template.replace('{id}', appId)));

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

export const GameCover = ({ appId, name, canonicalUrl }: GameCoverProps) => {
  const appIdString = String(appId);
  const urls = useMemo(
    () => buildCoverUrls(appIdString, canonicalUrl),
    [appIdString, canonicalUrl]
  );
  const [resolvedCover, setResolvedCover] = useState<ResolvedCover | null>(null);
  const [hasFinishedLookup, setHasFinishedLookup] = useState(false);

  useEffect(() => {
    setResolvedCover(null);
    setHasFinishedLookup(false);

    return preloadCoverChain(urls, (cover) => {
      setResolvedCover(cover);
      setHasFinishedLookup(true);
      if (cover) {
        saveCachedCover(appIdString, cover.url);
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
          loading="lazy"
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
