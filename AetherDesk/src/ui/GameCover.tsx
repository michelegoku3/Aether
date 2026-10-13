import { useEffect, useMemo, useRef, useState } from 'react';
import { cacheIsFinal, hasCoverChain, subscribeToCoverChain } from './coverChains';
import { buildCoverUrls } from './coverUrls';
import { getCachedCover } from './coverCache';
import type { ResolvedCover } from './coverTypes';

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
//   • ONE resolution chain per appId (single-flight registry): mounted card
//     and grid preload share the same work, and the last unsubscribing
//     subscriber really cancels the chain, request included.
//   • Lazy resolution (IntersectionObserver): no probes for off-viewport cards
//     or cards inside a hidden tab with `display:none`.
//   • A single cache read per mount (used to be three).
//
// Implementation lives in focused modules: `coverCache` (LRU + TTL + sweep),
// `coverUrls` (allowlist + candidates), `coverChains` (single-flight). This
// file owns only the component, its lazy lifecycle, and the grid preload.
// ────────────────────────────────────────────────────────────────────────────

interface GameCoverProps {
  appId: string | number;
  name: string;
  canonicalUrl?: string;
}

export interface GameCoverPreloadInput {
  appId: string | number;
  imageUrl?: string;
}

/** Warms the cache for a batch of games (called by the grids).
 *  Shares the single-flight registry with `GameCover`: when a card is already
 *  mounted — or the preload already passed by — no second chain starts. */
export const preloadGameCovers = (games: GameCoverPreloadInput[], maxCount = 40) => {
  games.slice(0, maxCount).forEach((game) => {
    const appIdString = String(game.appId);
    if (hasCoverChain(appIdString)) return;
    const urls = buildCoverUrls(appIdString, game.imageUrl, getCachedCover(appIdString)?.url);
    if (cacheIsFinal(getCachedCover(appIdString), urls)) return;
    // Preload has no lifecycle of its own: it detaches when the chain ends (at
    // which point the entry is already removed from the registry, so it is a
    // no-op). The container exists because the release function only exists
    // AFTER the subscribe, which in turn receives the listener.
    const release: { current?: () => void } = {};
    release.current = subscribeToCoverChain(appIdString, urls, () => {
      release.current?.();
    });
  });
};

const LAZY_ROOT_MARGIN = '256px';

export const GameCover = ({ appId, name, canonicalUrl }: GameCoverProps) => {
  const appIdString = String(appId);
  const urls = useMemo(
    () => buildCoverUrls(appIdString, canonicalUrl, getCachedCover(appIdString)?.url),
    [appIdString, canonicalUrl]
  );

  // ONE cache read per mount. Two state initializers used to call
  // `initialCachedCover` (localStorage.getItem + JSON.parse) each, and the
  // effect re-read it a third time: synchronous I/O in the first paint,
  // multiplied by every grid card.
  const [lookup, setLookup] = useState<{ cover: ResolvedCover | null; finished: boolean }>(() => {
    const cover = getCachedCover(appIdString);
    return { cover, finished: Boolean(cover) };
  });
  const resolvedCover = lookup.cover;

  // Lazy start: the chain only starts when the card is (nearly) in viewport.
  // Grids keep dozens of cards mounted and Store/Library stay mounted even on
  // a hidden tab (`display:none`), where the observer never fires: until the
  // user looks at that tab, work is zero.
  const wrapperRef = useRef<HTMLDivElement | null>(null);
  const [isNearViewport, setIsNearViewport] = useState(false);

  useEffect(() => {
    if (isNearViewport) return;
    const node = wrapperRef.current;
    if (!node) return;
    if (typeof IntersectionObserver === 'undefined') {
      setIsNearViewport(true); // no observer available: eager behavior
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setIsNearViewport(true);
          observer.disconnect();
        }
      },
      { rootMargin: LAZY_ROOT_MARGIN }
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, [isNearViewport]);

  // Cache re-read only when appId changes: the initializer already handled
  // the mount (and this is where the triple read used to happen).
  const loadedAppIdRef = useRef(appIdString);
  useEffect(() => {
    if (loadedAppIdRef.current === appIdString) return;
    loadedAppIdRef.current = appIdString;
    const cachedCover = getCachedCover(appIdString);
    setLookup({ cover: cachedCover, finished: Boolean(cachedCover) });
  }, [appIdString]);

  useEffect(() => {
    if (!isNearViewport) return;
    // `getCachedCover` here is an in-memory LRU hit (the initializer populated
    // it): it costs a lookup, not disk access.
    const cachedCover = getCachedCover(appIdString);
    if (cacheIsFinal(cachedCover, urls)) {
      setLookup((current) =>
        current.cover?.url === cachedCover?.url && current.finished ? current : { cover: cachedCover, finished: true }
      );
      return;
    }
    return subscribeToCoverChain(appIdString, urls, (cover) => {
      setLookup({ cover, finished: true });
    });
  }, [appIdString, urls, isNearViewport]);

  return (
    <div ref={wrapperRef} className={`game-cover-wrapper ${resolvedCover?.fit === 'landscape' ? 'landscape' : ''}`}>
      {resolvedCover?.fit === 'landscape' ? (
        <img src={resolvedCover.url} alt="" className="game-cover-backdrop" aria-hidden="true" loading="lazy" />
      ) : null}

      {resolvedCover ? (
        <img src={resolvedCover.url} alt={name} className={`game-cover-image ${resolvedCover.fit}`} loading="lazy" />
      ) : null}

      {!resolvedCover ? (
        <div className={`game-cover-fallback ${lookup.finished ? 'not-found' : 'loading'}`}>
          <span>Æ</span>
        </div>
      ) : null}
    </div>
  );
};
