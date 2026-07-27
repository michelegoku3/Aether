import { useEffect, useMemo, useState } from 'react';

const HERO_CACHE_PREFIX = 'aether_hero_';
const MIN_HERO_WIDTH = 300;
const MIN_HERO_HEIGHT = 120;

const HERO_TEMPLATES = [
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

interface GameHeroImageProps {
  appId: string | number;
  name: string;
  canonicalUrl?: string;
}

const getCachedHero = (appId: string) => {
  try { return localStorage.getItem(`${HERO_CACHE_PREFIX}${appId}`) || null; } catch { return null; }
};

const saveCachedHero = (appId: string, url: string) => {
  try { localStorage.setItem(`${HERO_CACHE_PREFIX}${appId}`, url); } catch {}
};

const buildHeroUrls = (appId: string, canonicalUrl?: string) => {
  const seen = new Set<string>();
  const urls: string[] = [];
  const push = (url?: string | null) => {
    const normalized = url?.trim().split('?')[0];
    if (!normalized || seen.has(normalized)) return;
    seen.add(normalized);
    urls.push(normalized);
  };

  push(getCachedHero(appId));
  push(canonicalUrl);
  HERO_TEMPLATES.forEach(template => push(template.replace('{id}', appId)));
  return urls;
};

const preloadHeroChain = (urls: string[], onResolved: (url: string | null) => void) => {
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
      if (image.naturalWidth < MIN_HERO_WIDTH || image.naturalHeight < MIN_HERO_HEIGHT) {
        index += 1;
        tryNext();
        return;
      }
      onResolved(url);
    };
    image.onerror = () => {
      index += 1;
      tryNext();
    };
    image.src = url;
  };

  tryNext();
  return () => { cancelled = true; };
};

export const GameHeroImage = ({ appId, name, canonicalUrl }: GameHeroImageProps) => {
  const appIdString = String(appId);
  const urls = useMemo(() => buildHeroUrls(appIdString, canonicalUrl), [appIdString, canonicalUrl]);
  const [resolvedUrl, setResolvedUrl] = useState<string | null>(null);

  useEffect(() => {
    setResolvedUrl(null);
    return preloadHeroChain(urls, (url) => {
      setResolvedUrl(url);
      if (url) saveCachedHero(appIdString, url);
    });
  }, [appIdString, urls]);

  return (
    <div className="game-action-hero">
      {resolvedUrl ? (
        <img src={resolvedUrl} alt={name} className="game-action-hero-image" />
      ) : (
        <div className="game-action-hero-fallback">Æ</div>
      )}
      <div className="game-action-hero-overlay">
        <span>{name}</span>
      </div>
    </div>
  );
};
