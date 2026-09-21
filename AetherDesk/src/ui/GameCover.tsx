import { useEffect, useMemo, useRef, useState } from 'react';

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
//   • UNA sola catena di risoluzione per appId (registro single-flight): card
//     montata e preload della griglia condividono lo stesso lavoro, e l'ultimo
//     sottoscrittore che si stacca annulla davvero la catena, request inclusa.
//   • Risoluzione lazy (IntersectionObserver): niente probe per le card fuori
//     viewport o dentro una tab nascosta con `display:none`.
//   • Una sola lettura di cache per mount (prima erano tre).
// ────────────────────────────────────────────────────────────────────────────

const COVER_CACHE_PREFIX = 'aether_cover_v4_';
const MIN_USABLE_WIDTH = 120;
const MIN_USABLE_HEIGHT = 60;
const PORTRAIT_RATIO_THRESHOLD = 0.85;

/** Cover-cache policy. Entries older than TTL are transparently discarded on
 *  read so we never re-render a months-old Steam capsule that may have been
 *  replaced. The in-memory LRU is bounded to avoid unbounded growth across
 *  long sessions browsing thousands of pages, and localStorage is swept when
 *  it exceeds MAX_PERSISTED_ENTRIES. */
const COVER_CACHE_TTL_MS = 30 * 24 * 60 * 60 * 1000; // 30 days
const MAX_MEMORY_ENTRIES = 500;
const MAX_PERSISTED_ENTRIES = 200;
const LEGACY_COVER_CACHE_PREFIX = 'aether_cover_v3_';

type CoverFit = 'portrait' | 'landscape';

interface ResolvedCover {
  url: string;
  fit: CoverFit;
}

interface PersistedCoverEnvelope {
  v: 4;
  url: string;
  fit: CoverFit;
  /** Epoch ms at which this entry was written. */
  cachedAt: number;
}

interface GameCoverProps {
  appId: string | number;
  name: string;
  canonicalUrl?: string;
}

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
const isCoverAssetUrl = (url: string): boolean => {
  const lower = url.toLowerCase();
  // `capsule_` alone matches `hero_capsule` (a landscape banner) — exclude.
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

/** Bounded LRU map for in-memory cover resolution. `get` promotes the key to
 *  most-recent so eviction drops the least-recently-used; `set` inserts and
 *  trims to MAX_MEMORY_ENTRIES. This replaces the previous unbounded `Map`
 *  (which grew without limit across long sessions). */
class BoundedCoverCache {
  private readonly map = new Map<string, ResolvedCover>();

  get(key: string): ResolvedCover | undefined {
    const value = this.map.get(key);
    if (value === undefined) return undefined;
    this.map.delete(key);
    this.map.set(key, value);
    return value;
  }

  set(key: string, value: ResolvedCover) {
    if (this.map.has(key)) this.map.delete(key);
    this.map.set(key, value);
    while (this.map.size > MAX_MEMORY_ENTRIES) {
      const oldestKey = this.map.keys().next().value;
      if (oldestKey === undefined) break;
      this.map.delete(oldestKey);
    }
  }
}

const memoryCoverCache = new BoundedCoverCache();

let haveSweptLegacyKeys = false;

/** One-time sweep: remove v3 (no TTL, raw URL/JSON) entries so they don't
 *  waste localStorage after the bump to v4. Lazy — runs on the first read. */
const sweepLegacyAndStaleEntries = () => {
  if (haveSweptLegacyKeys) return;
  haveSweptLegacyKeys = true;
  try {
    const toRemove: string[] = [];
    const kept: { key: string; cachedAt: number }[] = [];
    const now = Date.now();
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (!key) continue;
      if (key.startsWith(LEGACY_COVER_CACHE_PREFIX)) {
        toRemove.push(key);
        continue;
      }
      if (!key.startsWith(COVER_CACHE_PREFIX)) continue;
      // Keep v4 entries whose TTL is still valid; drop anything expired.
      try {
        const raw = localStorage.getItem(key);
        if (!raw) { toRemove.push(key); continue; }
        const env = JSON.parse(raw) as Partial<PersistedCoverEnvelope>;
        if (env.v !== 4 || typeof env.cachedAt !== 'number' || !env.url
            || !isCoverAssetUrl(env.url)
            || now - env.cachedAt > COVER_CACHE_TTL_MS) {
          toRemove.push(key);
          continue;
        }
        kept.push({ key, cachedAt: env.cachedAt });
      } catch {
        toRemove.push(key);
      }
    }
    toRemove.forEach((k) => { try { localStorage.removeItem(k); } catch { /* ignore */ } });
    // If we're still over the persisted cap, evict the oldest entries.
    if (kept.length > MAX_PERSISTED_ENTRIES) {
      kept.sort((a, b) => a.cachedAt - b.cachedAt);
      kept.slice(MAX_PERSISTED_ENTRIES).forEach(({ key }) => {
        try { localStorage.removeItem(key); } catch { /* ignore */ }
      });
    }
  } catch {
    // localStorage unavailable (private mode / quota) — nothing to do.
  }
};

const parseCachedCover = (raw: string | null): ResolvedCover | null => {
  if (!raw) return null;
  try {
    const env = JSON.parse(raw) as Partial<PersistedCoverEnvelope>;
    if (env.v === 4
        && typeof env.url === 'string'
        && (env.fit === 'portrait' || env.fit === 'landscape')
        && typeof env.cachedAt === 'number'
        && Date.now() - env.cachedAt <= COVER_CACHE_TTL_MS
        && isCoverAssetUrl(env.url)) {
      return { url: env.url, fit: env.fit };
    }
  } catch {
    // Fall through: v3 entries or plain strings are rejected by the legacy
    // sweep on first read. We return null instead of guessing to stay safe.
  }
  return null;
};

const getCachedCover = (appId: string): ResolvedCover | null => {
  sweepLegacyAndStaleEntries();
  const memoryHit = memoryCoverCache.get(appId);
  if (memoryHit) return memoryHit;

  try {
    const raw = localStorage.getItem(`${COVER_CACHE_PREFIX}${appId}`);
    const parsed = parseCachedCover(raw);
    if (parsed) {
      memoryCoverCache.set(appId, parsed);
    } else if (raw) {
      // Defensive: clear any unparseable/expired entry we just read.
      localStorage.removeItem(`${COVER_CACHE_PREFIX}${appId}`);
    }
    return parsed;
  } catch {
    return null;
  }
};

const saveCachedCover = (appId: string, cover: ResolvedCover) => {
  memoryCoverCache.set(appId, cover);
  try {
    const env: PersistedCoverEnvelope = {
      v: 4,
      url: cover.url,
      fit: cover.fit,
      cachedAt: Date.now(),
    };
    localStorage.setItem(`${COVER_CACHE_PREFIX}${appId}`, JSON.stringify(env));
  } catch {
    // Cache is an optimization only; ignore storage quota/privacy errors.
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

const preloadCoverChain = (
  urls: string[],
  onResolved: (cover: ResolvedCover | null) => void,
): (() => void) => {
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
    // Interrompe anche la request in volo. Prima il cancel fermava solo i
    // callback: il download continuava comunque, e con 9 URL per card su griglie
    // da 20+ card il costo in banda non era trascurabile.
    if (currentImage) {
      currentImage.onload = null;
      currentImage.onerror = null;
      currentImage.src = '';
      currentImage = null;
    }
  };
};

// ---------------------------------------------------------------------------
// Single-flight: UNA catena di risoluzione per appId
// ---------------------------------------------------------------------------
//
// Senza questo registro la stessa cover veniva risolta più volte in parallelo:
// il preload della griglia avviava una catena e ogni `GameCover` montato ne
// avviava un'altra (React StrictMode monta due volte; Store e Library possono
// mostrare lo stesso gioco). Ogni catena prova fino a 9 URL con `new Image()`,
// quindi N catene duplicate significavano N×9 request ai CDN di Steam per la
// stessa identica immagine.
//
// Ora i sottoscrittori si agganciano alla catena esistente. L'ultimo che si
// stacca la cancella davvero — interrompendo anche la request in volo — così un
// cambio filtro o uno scroll veloce non lasciano code di catene orfane.

type CoverListener = (cover: ResolvedCover | null) => void;

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

/** Avvia la catena per `appId` o si aggancia a quella già in corso.
 *  Restituisce la funzione di rilascio (da usare come cleanup di un effect). */
const subscribeToCoverChain = (
  appId: string,
  urls: string[],
  listener: CoverListener,
): (() => void) => {
  const existing = coverChains.get(appId);
  if (existing) {
    existing.listeners.add(listener);
    return () => releaseCoverChain(appId, listener);
  }
  const chain: CoverChain = { listeners: new Set([listener]), cancel: () => {} };
  coverChains.set(appId, chain);
  chain.cancel = preloadCoverChain(urls, (cover) => {
    // Fuori dal registro PRIMA di notificare: un sottoscrittore che riprova
    // subito (es. `canonicalUrl` arrivato in ritardo) riparte pulito invece di
    // agganciarsi a una catena già risolta.
    coverChains.delete(appId);
    if (cover) saveCachedCover(appId, cover);
    for (const current of Array.from(chain.listeners)) current(cover);
    chain.listeners.clear();
  });
  return () => releaseCoverChain(appId, listener);
};

/** True quando la cache contiene già la risposta definitiva: la sua URL è la
 *  prima che la catena proverebbe, quindi una riverifica costerebbe una request
 *  per ottenere lo stesso risultato. */
const cacheIsFinal = (cached: ResolvedCover | null, urls: string[]): boolean =>
  Boolean(cached && urls[0] && cached.url === urls[0]);

export interface GameCoverPreloadInput {
  appId: string | number;
  imageUrl?: string;
}

/** Riscalda la cache per un blocco di giochi (chiamato dalle griglie).
 *  Condivide il registro single-flight con `GameCover`: se una card è già
 *  montata — o il preload è già passato di lì — non parte una seconda catena. */
export const preloadGameCovers = (games: GameCoverPreloadInput[], maxCount = 40) => {
  games.slice(0, maxCount).forEach((game) => {
    const appIdString = String(game.appId);
    if (coverChains.has(appIdString)) return;
    const urls = buildCoverUrls(appIdString, game.imageUrl);
    if (cacheIsFinal(getCachedCover(appIdString), urls)) return;
    // Il preload non ha un ciclo di vita proprio: si stacca a catena finita (a
    // quel punto l'entry è già rimossa dal registro, quindi è un no-op). Il
    // contenitore serve perché la funzione di rilascio esiste solo DOPO la
    // subscribe, che a sua volta riceve il listener.
    const release: { current?: () => void } = {};
    release.current = subscribeToCoverChain(appIdString, urls, () => {
      release.current?.();
    });
  });
};

/** Distanza dal viewport a cui la risoluzione lazy si mette in moto: abbastanza
 *  da non mostrare il placeholder durante uno scroll normale. */
const LAZY_ROOT_MARGIN = '256px';

export const GameCover = ({ appId, name, canonicalUrl }: GameCoverProps) => {
  const appIdString = String(appId);
  const urls = useMemo(
    () => buildCoverUrls(appIdString, canonicalUrl),
    [appIdString, canonicalUrl],
  );

  // UNA lettura di cache per mount. Prima due inizializzatori di stato
  // chiamavano `initialCachedCover` (localStorage.getItem + JSON.parse) a testa
  // e l'effect la rileggeva una terza volta: I/O sincrono nel primo paint,
  // moltiplicato per ogni card della griglia.
  const [lookup, setLookup] = useState<{ cover: ResolvedCover | null; finished: boolean }>(() => {
    const cover = getCachedCover(appIdString);
    return { cover, finished: Boolean(cover) };
  });
  const resolvedCover = lookup.cover;

  // Lazy start: la catena parte solo quando la card è (quasi) in viewport.
  // Le griglie tengono montate decine di card e Store/Library restano montate
  // anche a tab nascosta (`display:none`), dove l'observer non scatta: finché
  // l'utente non guarda quella tab, il lavoro è zero.
  const wrapperRef = useRef<HTMLDivElement | null>(null);
  const [isNearViewport, setIsNearViewport] = useState(false);

  useEffect(() => {
    if (isNearViewport) return;
    const node = wrapperRef.current;
    if (!node) return;
    if (typeof IntersectionObserver === 'undefined') {
      setIsNearViewport(true); // nessun observer disponibile: comportamento eager
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setIsNearViewport(true);
          observer.disconnect();
        }
      },
      { rootMargin: LAZY_ROOT_MARGIN },
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, [isNearViewport]);

  // Rilettura della cache solo se cambia appId: al mount ci ha già pensato
  // l'inizializzatore (ed è qui che prima avveniva la lettura tripla).
  const loadedAppIdRef = useRef(appIdString);
  useEffect(() => {
    if (loadedAppIdRef.current === appIdString) return;
    loadedAppIdRef.current = appIdString;
    const cachedCover = getCachedCover(appIdString);
    setLookup({ cover: cachedCover, finished: Boolean(cachedCover) });
  }, [appIdString]);

  useEffect(() => {
    if (!isNearViewport) return;
    // `getCachedCover` a questo punto è una hit della LRU in memoria (l'ha
    // popolata l'inizializzatore): costa un lookup, non un accesso a disco.
    const cachedCover = getCachedCover(appIdString);
    if (cacheIsFinal(cachedCover, urls)) {
      setLookup((current) =>
        current.cover?.url === cachedCover?.url && current.finished
          ? current
          : { cover: cachedCover, finished: true },
      );
      return;
    }
    return subscribeToCoverChain(appIdString, urls, (cover) => {
      setLookup({ cover, finished: true });
    });
  }, [appIdString, urls, isNearViewport]);

  return (
    <div
      ref={wrapperRef}
      className={`game-cover-wrapper ${resolvedCover?.fit === 'landscape' ? 'landscape' : ''}`}
    >
      {resolvedCover?.fit === 'landscape' ? (
        <img src={resolvedCover.url} alt="" className="game-cover-backdrop" aria-hidden="true" loading="lazy" />
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
        <div className={`game-cover-fallback ${lookup.finished ? 'not-found' : 'loading'}`}>
          <span>Æ</span>
        </div>
      ) : null}
    </div>
  );
};
