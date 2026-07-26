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

  // Then try Steam's newer wide library/header/capsule assets. These are not stretched:
  // GameCover classifies them at load time and renders them with contain + blurred backdrop.
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

type CoverFit = 'unknown' | 'portrait' | 'landscape';

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

const classifyCover = (image: HTMLImageElement): CoverFit | null => {
  const { naturalWidth, naturalHeight } = image;

  // Some Steam endpoints can return tiny placeholder-like images. Skip those instead
  // of caching/rendering them, because they look pixelated in the card.
  if (naturalWidth < MIN_USABLE_WIDTH || naturalHeight < MIN_USABLE_HEIGHT) {
    return null;
  }

  const ratio = naturalWidth / naturalHeight;
  return ratio <= PORTRAIT_RATIO_THRESHOLD ? 'portrait' : 'landscape';
};

export const GameCover = ({ appId, name, canonicalUrl }: GameCoverProps) => {
  const appIdString = String(appId);
  const urls = useMemo(
    () => buildCoverUrls(appIdString, canonicalUrl),
    [appIdString, canonicalUrl]
  );
  const [urlIndex, setUrlIndex] = useState(0);
  const [coverFit, setCoverFit] = useState<CoverFit>('unknown');
  const currentUrl = urls[urlIndex];

  useEffect(() => {
    setUrlIndex(0);
    setCoverFit('unknown');
  }, [urls]);

  const tryNextUrl = () => {
    setCoverFit('unknown');
    setUrlIndex(index => index + 1);
  };

  return (
    <div className={`game-cover-wrapper ${coverFit === 'landscape' ? 'landscape' : ''}`}>
      {currentUrl && coverFit === 'landscape' ? (
        <img src={currentUrl} alt="" className="game-cover-backdrop" aria-hidden="true" />
      ) : null}

      {currentUrl ? (
        <img
          src={currentUrl}
          alt={name}
          className={`game-cover-image ${coverFit}`}
          loading="lazy"
          onLoad={(event) => {
            const nextFit = classifyCover(event.currentTarget);
            if (!nextFit) {
              tryNextUrl();
              return;
            }

            setCoverFit(nextFit);
            saveCachedCover(appIdString, event.currentTarget.src);
          }}
          onError={tryNextUrl}
        />
      ) : null}
      <div className="game-cover-fallback">
        <span>Æ</span>
      </div>
    </div>
  );
};
