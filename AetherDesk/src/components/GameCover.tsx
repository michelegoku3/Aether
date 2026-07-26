import { useMemo, useState } from 'react';

const COVER_CACHE_PREFIX = 'aether_cover_';

const STEAM_COVER_TEMPLATES = [
  // Same reliability principle used by SFF: try current shared CDN first,
  // then older CDN aliases and header/capsule shapes.
  'https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{id}/library_600x900.jpg',
  'https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{id}/library_600x900.jpg',
  'https://shared.steamstatic.com/store_item_assets/steam/apps/{id}/library_600x900.jpg',
  'https://cdn.cloudflare.steamstatic.com/steam/apps/{id}/library_600x900.jpg',
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

export const GameCover = ({ appId, name, canonicalUrl }: GameCoverProps) => {
  const appIdString = String(appId);
  const urls = useMemo(
    () => buildCoverUrls(appIdString, canonicalUrl),
    [appIdString, canonicalUrl]
  );
  const [urlIndex, setUrlIndex] = useState(0);
  const currentUrl = urls[urlIndex];

  return (
    <div className="game-cover-wrapper">
      {currentUrl ? (
        <img
          src={currentUrl}
          alt={name}
          className="game-cover-image"
          loading="lazy"
          onLoad={(event) => saveCachedCover(appIdString, event.currentTarget.src)}
          onError={() => setUrlIndex(index => index + 1)}
        />
      ) : null}
      <div className="game-cover-fallback">
        <span>Æ</span>
      </div>
    </div>
  );
};
