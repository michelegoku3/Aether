import { saveCachedCover } from './coverCache';
import type { ResolvedCover } from './coverTypes';
import type { CoverFit } from './coverTypes';

const MIN_USABLE_WIDTH = 120;
const MIN_USABLE_HEIGHT = 60;
const PORTRAIT_RATIO_THRESHOLD = 0.85;

const classifyLoadedImage = (image: HTMLImageElement): CoverFit | null => {
  const { naturalWidth, naturalHeight } = image;
  if (naturalWidth < MIN_USABLE_WIDTH || naturalHeight < MIN_USABLE_HEIGHT) {
    return null;
  }
  const ratio = naturalWidth / naturalHeight;
  return ratio <= PORTRAIT_RATIO_THRESHOLD ? 'portrait' : 'landscape';
};

const preloadCoverChain = (urls: string[], onResolved: (cover: ResolvedCover | null) => void): (() => void) => {
  let cancelled = false;
  let index = 0;
  let currentImage: HTMLImageElement | null = null;

  const tryNext = () => {
    if (cancelled) return;
    const url = urls[index];
    if (!url) {
      onResolved(null);
      return;
    }
    const image = new Image();
    image.decoding = 'async';
    currentImage = image;
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
      if (cancelled) return;
      index += 1;
      tryNext();
    };
    image.src = url;
  };

  tryNext();

  return () => {
    cancelled = true;
    // Also aborts the in-flight request. Cancel used to stop only the
    // callbacks: the download kept going anyway, and with 9 URLs per card on
    // 20+ card grids the bandwidth cost was not negligible.
    if (currentImage) {
      currentImage.onload = null;
      currentImage.onerror = null;
      currentImage.src = '';
      currentImage = null;
    }
  };
};

// ---------------------------------------------------------------------------
// Single-flight: ONE resolution chain per appId
// ---------------------------------------------------------------------------
//
// Without this registry the same cover was resolved several times in parallel:
// the grid preload started one chain and every mounted `GameCover` started
// another (React StrictMode mounts twice; Store and Library can show the same
// game). Each chain tries up to 9 URLs with `new Image()`, so N duplicated
// chains meant N×9 requests to the Steam CDNs for the very same image.
//
// Now subscribers hook onto the existing chain. The last one to detach really
// cancels it — aborting the in-flight request too — so a filter change or a
// fast scroll leaves no queues of orphaned chains.

export type CoverListener = (cover: ResolvedCover | null) => void;

interface CoverChain {
  listeners: Set<CoverListener>;
  cancel: () => void;
}

const coverChains = new Map<string, CoverChain>();

const releaseCoverChain = (appId: string, listener: CoverListener) => {
  const chain = coverChains.get(appId);
  if (!chain) return;
  chain.listeners.delete(listener);
  if (chain.listeners.size > 0) return;
  chain.cancel();
  coverChains.delete(appId);
};

/** True when a resolution chain is already running for `appId`. */
export const hasCoverChain = (appId: string): boolean => coverChains.has(appId);

/** Starts the chain for `appId` or hooks onto the one already running.
 *  Returns the release function (to use as an effect cleanup). */
export const subscribeToCoverChain = (appId: string, urls: string[], listener: CoverListener): (() => void) => {
  const existing = coverChains.get(appId);
  if (existing) {
    existing.listeners.add(listener);
    return () => releaseCoverChain(appId, listener);
  }
  const chain: CoverChain = { listeners: new Set([listener]), cancel: () => {} };
  coverChains.set(appId, chain);
  chain.cancel = preloadCoverChain(urls, (cover) => {
    // Out of the registry BEFORE notifying: a subscriber retrying right away
    // (e.g. a late-arriving `canonicalUrl`) restarts clean instead of hooking
    // onto an already-resolved chain.
    coverChains.delete(appId);
    if (cover) saveCachedCover(appId, cover);
    for (const current of Array.from(chain.listeners)) current(cover);
    chain.listeners.clear();
  });
  return () => releaseCoverChain(appId, listener);
};

/** True when the cache already holds the final answer: its URL is the first
 *  the chain would try, so re-verifying would cost a request to get the same
 *  result. */
export const cacheIsFinal = (cached: ResolvedCover | null, urls: string[]): boolean =>
  Boolean(cached && urls[0] && cached.url === urls[0]);
