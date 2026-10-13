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
export const isCoverAssetUrl = (url: string): boolean => {
  const lower = url.toLowerCase();
  // `capsule_` alone matches `hero_capsule` (a landscape banner) — exclude.
  if (lower.includes('hero_capsule') || lower.includes('library_hero') || lower.includes('/header')) {
    return false;
  }
  return (
    lower.includes('library_capsule') ||
    lower.includes('main_capsule') ||
    lower.includes('small_capsule') ||
    lower.includes('capsule_231x87') ||
    lower.includes('capsule_616x353') ||
    lower.includes('library_600x900')
  );
};

const normalizeCanonicalUrl = (url?: string): string | null => {
  const trimmed = url?.trim();
  if (!trimmed) return null;
  return trimmed.split('?')[0];
};

// Build the ordered list of cover candidates. Only cover-shaped URLs survive;
// hero/header/background URLs are dropped here, before any fetch happens.
// `cachedUrl` is passed in (read by the caller) so this module stays free of
// cache imports.
export const buildCoverUrls = (appId: string, canonicalUrl?: string, cachedUrl?: string | null): string[] => {
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
  push(cachedUrl || null);
  STEAM_CAPSULE_FALLBACK_TEMPLATES.forEach((template) => push(template.replace('{id}', appId)));

  return urls;
};
