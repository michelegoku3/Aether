import type { GameInfoPlatforms, GameInfoPrice, GameInfoScreenshot } from './gameInfoTypes';

export const NA = 'N/A';

const CONTENT_DESCRIPTOR_LABELS: Record<number, string> = {
  1: 'Some Nudity or Sexual Content',
  2: 'Frequent Violence or Gore',
  3: 'Frequent Nudity or Sexual Content',
  4: 'Adult Only Sexual Content',
  5: 'General Mature Content',
};

export const yesNo = (value?: boolean | null) => {
  if (value === true) return 'Yes';
  if (value === false) return 'No';
  return NA;
};

export const formatUnixDate = (value?: number | null) => {
  if (!value) return NA;
  return new Date(value * 1000).toLocaleDateString();
};

export const formatList = (items?: string[]) => {
  const clean = (items || []).map((item) => item.trim()).filter(Boolean);
  return clean.length > 0 ? clean.join(', ') : NA;
};

export const platformSummary = (platforms?: GameInfoPlatforms | null) => {
  if (!platforms) return NA;
  const items = [
    platforms.windows ? 'Windows' : null,
    platforms.mac ? 'macOS' : null,
    platforms.linux ? 'Linux' : null,
    platforms.hasVrSupport ? 'VR' : null,
  ].filter(Boolean);
  return items.length > 0 ? items.join(', ') : NA;
};

export const priceSummary = (price?: GameInfoPrice | null, isFree?: boolean | null) => {
  if (isFree) return 'Free to Play';
  if (!price) return NA;
  if (price.formattedFinal) return price.formattedFinal;
  if (typeof price.finalCents === 'number') {
    const currency = (price.currency || 'EUR').toUpperCase();
    if (currency === 'USD') return `$${(price.finalCents / 100).toFixed(2)}`;
    if (currency === 'EUR') return `€${(price.finalCents / 100).toFixed(2)}`;
    if (currency === 'JPY') return `¥${price.finalCents}`;
    return `${currency} ${(price.finalCents / 100).toFixed(2)}`;
  }
  return NA;
};

export const cacheAge = (timestamp?: number | null) => {
  if (!timestamp) return NA;
  const seconds = Math.max(0, Math.floor(Date.now() / 1000) - timestamp);
  if (seconds < 60) return 'Just now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ago`;
  return `${Math.floor(seconds / 86400)} d ago`;
};

export const displayKind = (kind?: string | null) => {
  const value = kind?.trim();
  if (!value) return NA;
  return value
    .replace(/[_-]+/g, ' ')
    .split(/\s+/)
    .map((word) => (word.toLowerCase() === 'dlc' ? 'DLC' : word.charAt(0).toUpperCase() + word.slice(1).toLowerCase()))
    .join(' ');
};

export const contentDescriptorSummary = (ids?: number[]) => {
  const labels = (ids || []).map((id) => CONTENT_DESCRIPTOR_LABELS[id] || `Descriptor ${id}`);
  return labels.length > 0 ? labels.join(', ') : NA;
};

const screenshotThumbnail = (shot: GameInfoScreenshot) =>
  shot.thumbnail || shot.pathThumbnail || shot.path_thumbnail || shot.full || shot.pathFull || shot.path_full || '';

const screenshotFull = (shot: GameInfoScreenshot) =>
  shot.full || shot.pathFull || shot.path_full || shot.thumbnail || shot.pathThumbnail || shot.path_thumbnail || '';

export const decodeHtmlEntities = (value?: string | null) => {
  if (!value) return '';
  let decoded = value;
  for (let i = 0; i < 3; i += 1) {
    const textarea = document.createElement('textarea');
    textarea.innerHTML = decoded;
    const next = textarea.value;
    if (next === decoded) break;
    decoded = next;
  }
  return decoded;
};

export const normalizeScreenshots = (...groups: Array<GameInfoScreenshot[] | undefined>) => {
  const seen = new Set<string>();
  return groups
    .flatMap((group) => group || [])
    .map((shot) => ({ ...shot, thumbnail: screenshotThumbnail(shot), full: screenshotFull(shot) }))
    .filter((shot) => {
      const key = shot.full || shot.thumbnail;
      if (!key || seen.has(key)) return false;
      seen.add(key);
      return true;
    });
};
