import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { GameCover } from '../ui/GameCover';
import type { GameInfo, GameInfoModalProps } from './gameInfoTypes';
import {
  cacheAge,
  contentDescriptorSummary,
  decodeHtmlEntities,
  displayKind,
  formatList,
  formatUnixDate,
  NA,
  normalizeScreenshots,
  platformSummary,
  priceSummary,
  yesNo,
} from './gameInfoFormat';
import { InfoRow, InfoSection } from './InfoSections';

export const GameInfoModal = ({ appId, fallbackName, fallbackImageUrl, onClose }: GameInfoModalProps) => {
  const [info, setInfo] = useState<GameInfo | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState('');
  const [selectedScreenshotIndex, setSelectedScreenshotIndex] = useState<number | null>(null);
  const [isEdgeZone, setIsEdgeZone] = useState(false);
  // Full-size URLs that finished loading in the lightbox: while the current
  // one is still in flight the (already cached) thumbnail is shown in its
  // place, so switching never looks frozen.
  const [loadedFullSrcs, setLoadedFullSrcs] = useState<Record<string, true>>({});
  const [failedFullSrcs, setFailedFullSrcs] = useState<Record<string, true>>({});
  // Intrinsic size per URL (thumbnail or full): lets the lightbox stage take
  // the final aspect ratio immediately, so odd (square/portrait) screenshots
  // are framed correctly and nothing jumps when the full image arrives.
  const [imageDims, setImageDims] = useState<Record<string, readonly [number, number]>>({});
  // Keeps the preloaded neighbours alive (the browser cache does the rest).
  const preloadedRef = useRef<Map<string, HTMLImageElement>>(new Map());

  useEffect(() => {
    let cancelled = false;
    setIsLoading(true);
    setError('');

    invoke<GameInfo>('get_game_info', { appId })
      .then((result) => {
        if (cancelled) return;
        setInfo(result);
      })
      .catch((err) => {
        if (cancelled) return;
        setError(String(err));
      })
      .finally(() => {
        if (cancelled) return;
        setIsLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [appId]);

  const title = info?.name || fallbackName;
  const imageUrl = info?.imageUrl || info?.appDetails?.capsuleImage || fallbackImageUrl;
  const details = info?.appDetails;
  const local = info?.local;
  const screenshots = useMemo(
    () => normalizeScreenshots(info?.screenshots, details?.screenshots),
    [info?.screenshots, details?.screenshots]
  );
  const screenshotCount = screenshots.length;
  const shortDescription = decodeHtmlEntities(details?.shortDescription);

  const markFullLoaded = useCallback((src: string) => {
    if (!src) return;
    setLoadedFullSrcs((prev) => (prev[src] ? prev : { ...prev, [src]: true }));
  }, []);

  const markFullFailed = useCallback((src: string) => {
    if (!src) return;
    setFailedFullSrcs((prev) => (prev[src] ? prev : { ...prev, [src]: true }));
  }, []);

  const rememberDims = useCallback((src: string, img: HTMLImageElement | null) => {
    if (!src || !img || !img.naturalWidth || !img.naturalHeight) return;
    const width = img.naturalWidth;
    const height = img.naturalHeight;
    setImageDims((prev) => {
      const current = prev[src];
      if (current && current[0] === width && current[1] === height) return prev;
      return { ...prev, [src]: [width, height] as const };
    });
  }, []);

  const handlePrevScreenshot = useCallback(
    (e?: React.MouseEvent) => {
      e?.stopPropagation();
      if (screenshotCount === 0) return;
      setSelectedScreenshotIndex((prev) => (prev !== null ? (prev - 1 + screenshotCount) % screenshotCount : 0));
    },
    [screenshotCount]
  );

  const handleNextScreenshot = useCallback(
    (e?: React.MouseEvent) => {
      e?.stopPropagation();
      if (screenshotCount === 0) return;
      setSelectedScreenshotIndex((prev) => (prev !== null ? (prev + 1) % screenshotCount : 0));
    },
    [screenshotCount]
  );

  useEffect(() => {
    if (selectedScreenshotIndex === null) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setSelectedScreenshotIndex(null);
      } else if (event.key === 'ArrowLeft') {
        event.preventDefault();
        handlePrevScreenshot();
      } else if (event.key === 'ArrowRight') {
        event.preventDefault();
        handleNextScreenshot();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [selectedScreenshotIndex, handlePrevScreenshot, handleNextScreenshot]);

  // Warm the browser cache with the neighbours of the current screenshot so
  // Next/Prev only has to paint. Full-size Steam screenshots are large and
  // fetched on demand: without this every arrow press waited for the CDN.
  useEffect(() => {
    if (selectedScreenshotIndex === null || screenshotCount < 2) return;
    const preloaded = preloadedRef.current;
    for (const offset of [1, -1, 2]) {
      const shot = screenshots[(selectedScreenshotIndex + offset + screenshotCount) % screenshotCount];
      const src = shot?.full || shot?.thumbnail;
      if (!src || preloaded.has(src)) continue;
      const img = new Image();
      img.decoding = 'async';
      img.onload = () => {
        markFullLoaded(src);
        rememberDims(src, img);
      };
      img.src = src;
      preloaded.set(src, img);
      // Bound the retained set: the cache keeps the bytes anyway.
      if (preloaded.size > 12) {
        const oldest = preloaded.keys().next().value;
        if (oldest !== undefined) preloaded.delete(oldest);
      }
    }
  }, [selectedScreenshotIndex, screenshots, screenshotCount, markFullLoaded, rememberDims]);

  useEffect(() => {
    if (selectedScreenshotIndex === null) setIsEdgeZone(false);
  }, [selectedScreenshotIndex]);

  const activeShot = selectedScreenshotIndex !== null ? screenshots[selectedScreenshotIndex] : undefined;
  const activeFullSrc = activeShot?.full || activeShot?.thumbnail || '';
  const activeThumbSrc = activeShot?.thumbnail || activeShot?.full || '';
  const activeFullLoaded = !!activeFullSrc && !!loadedFullSrcs[activeFullSrc];
  const activeFullFailed = !!activeFullSrc && !!failedFullSrcs[activeFullSrc];
  const stageState = activeFullLoaded ? 'is-loaded' : activeFullFailed ? 'is-error' : 'is-loading';
  const activeDims = imageDims[activeFullSrc] || imageDims[activeThumbSrc];
  // Stage geometry: same aspect ratio as the screenshot, capped by the
  // viewport on both axes. Placeholder and full image share the same box.
  const stageStyle = activeDims
    ? {
        aspectRatio: `${activeDims[0]} / ${activeDims[1]}`,
        width: `min(1200px, 100%, calc(90vh * ${(activeDims[0] / activeDims[1]).toFixed(4)}))`,
      }
    : undefined;

  const cacheSummary = useMemo(() => {
    if (!info) return [];
    return [
      ['Store search', cacheAge(info.storeSearchUpdatedAtUnix)],
      ['Store metadata', cacheAge(info.storeItemsUpdatedAtUnix)],
      ['Steam appdetails', cacheAge(info.appdetailsUpdatedAtUnix)],
      ['Hubcap status', cacheAge(info.hubcapUpdatedAtUnix)],
      ['Local library', cacheAge(info.localUpdatedAtUnix)],
    ];
  }, [info]);

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-container info-modal-container" onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <span className="modal-title">
            Info: <strong style={{ color: '#ffffff' }}>{title}</strong> ({appId})
          </span>
          <button onClick={onClose} className="modal-close-btn">
            &times;
          </button>
        </div>

        <div className="modal-separator"></div>

        <div className="modal-body info-modal-body">
          {isLoading ? (
            <div className="info-loading">Loading game information...</div>
          ) : error ? (
            <div className="settings-alert error">Failed to load info: {error}</div>
          ) : info ? (
            <>
              <div className="info-hero">
                <GameCover appId={String(appId)} name={title} canonicalUrl={imageUrl || undefined} />
                <div className="info-hero-text">
                  <h2>{title}</h2>
                  {shortDescription && <p>{shortDescription}</p>}
                </div>
              </div>

              <div className="info-grid">
                <InfoSection title="Screenshots" className="info-section-full">
                  {screenshots.length > 0 ? (
                    <div className="info-screenshot-strip">
                      {screenshots.map((shot, index) => {
                        const src = shot.thumbnail || shot.full || '';
                        const alt = `${title} screenshot ${index + 1}`;
                        return (
                          <button
                            key={`${shot.id ?? index}-${src}`}
                            type="button"
                            className="info-screenshot-link"
                            onClick={() => setSelectedScreenshotIndex(index)}
                          >
                            <img
                              src={src}
                              alt={alt}
                              loading="lazy"
                              decoding="async"
                              onLoad={(event) => rememberDims(src, event.currentTarget)}
                            />
                          </button>
                        );
                      })}
                    </div>
                  ) : (
                    <InfoRow label="Screenshots" value={NA} />
                  )}
                </InfoSection>

                <InfoSection title="Store">
                  <InfoRow label="Type" value={displayKind(info.kind)} />
                  <InfoRow label="Price" value={priceSummary(info.price, details?.isFree)} />
                  <InfoRow label="Metascore" value={info.metascore || details?.metacriticScore} />
                  <InfoRow label="Release" value={details?.releaseDateText || formatUnixDate(info.releaseDateUnix)} />
                  <InfoRow label="Controller" value={displayKind(info.controllerSupport)} />
                  <InfoRow label="Platforms" value={platformSummary(info.platforms)} />
                </InfoSection>

                <InfoSection title="Availability & flags">
                  <InfoRow label="Manifest available" value={yesNo(info.hasManifest)} />
                  <InfoRow label="Denuvo" value={yesNo(info.hasDenuvo)} />
                  <InfoRow label="NSFW" value={yesNo(info.hasNsfw)} />
                  <InfoRow label="Delisted" value={yesNo(info.hasDelisted)} />
                  <InfoRow label="Content descriptors" value={contentDescriptorSummary(info.contentDescriptorIds)} />
                </InfoSection>

                <InfoSection title="Publisher data">
                  <InfoRow label="Publishers" value={formatList(details?.publishers)} />
                  <InfoRow label="Developers" value={formatList(details?.developers)} />
                  <InfoRow label="Genres" value={formatList(details?.genres)} />
                  <InfoRow label="Categories" value={formatList(details?.categories)} clamp />
                  <InfoRow label="Achievements" value={details?.achievementsTotal} />
                  <InfoRow label="Recommendations" value={details?.recommendationsTotal} />
                </InfoSection>

                <InfoSection title="Local library">
                  <InfoRow label="Lua installed" value={yesNo(local?.luaInstalled)} />
                  <InfoRow label="Game installed" value={yesNo(local?.installed)} />
                  <InfoRow label="Manifest pins" value={local?.manifestPinCount} />
                  <InfoRow label="Updates enabled" value={yesNo(local?.updatesEnabled)} />
                  <InfoRow label="Install dir" value={local?.installDir} />
                  <InfoRow label="Library path" value={local?.libraryPath} />
                </InfoSection>

                <InfoSection title="Cache freshness" className="info-section-full">
                  {cacheSummary.map(([label, value]) => (
                    <InfoRow key={label} label={label} value={value} />
                  ))}
                </InfoSection>
              </div>
            </>
          ) : null}
        </div>
      </div>

      {selectedScreenshotIndex !== null && activeShot && (
        <div
          className="info-lightbox"
          onMouseMove={(event) => {
            const w = window.innerWidth;
            const x = event.clientX;
            // First (0 - 25%) or fourth (75% - 100%) quadrant -> show both arrows
            setIsEdgeZone(x < w * 0.25 || x > w * 0.75);
          }}
          onMouseLeave={() => setIsEdgeZone(false)}
          onClick={(event) => {
            event.stopPropagation();
            setSelectedScreenshotIndex(null);
          }}
        >
          {/* Arrows live on the overlay (viewport edges), not inside the image
              box: a square/portrait screenshot makes that box narrow and the
              arrows used to land outside the 25% hover zones that reveal
              them, so they looked broken. */}
          {screenshotCount > 1 && (
            <>
              <button
                type="button"
                className={`info-lightbox-nav prev ${isEdgeZone ? 'visible' : ''}`}
                onClick={handlePrevScreenshot}
                aria-label="Previous screenshot"
                title="Previous screenshot"
              >
                <svg
                  width="48"
                  height="48"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2.5"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  aria-hidden="true"
                >
                  <polyline points="15 18 9 12 15 6" />
                </svg>
              </button>

              <button
                type="button"
                className={`info-lightbox-nav next ${isEdgeZone ? 'visible' : ''}`}
                onClick={handleNextScreenshot}
                aria-label="Next screenshot"
                title="Next screenshot"
              >
                <svg
                  width="48"
                  height="48"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2.5"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  aria-hidden="true"
                >
                  <polyline points="9 18 15 12 9 6" />
                </svg>
              </button>
            </>
          )}

          <div
            className={`info-lightbox-content ${stageState}`}
            style={stageStyle}
            onClick={(event) => event.stopPropagation()}
          >
            <button
              type="button"
              className="info-lightbox-close"
              onClick={() => setSelectedScreenshotIndex(null)}
              aria-label="Close screenshot preview"
            >
              &times;
            </button>

            {/* Instant placeholder: the strip thumbnail is already cached. */}
            {activeThumbSrc && activeThumbSrc !== activeFullSrc && (
              <img
                className="info-lightbox-placeholder"
                src={activeThumbSrc}
                alt=""
                aria-hidden="true"
                decoding="async"
                onLoad={(event) => rememberDims(activeThumbSrc, event.currentTarget)}
              />
            )}

            {/* Keyed by URL: a fresh element never keeps painting the previous
                screenshot while the new one downloads. */}
            <img
              key={activeFullSrc}
              className="info-lightbox-full"
              src={activeFullSrc}
              alt={`${title} screenshot ${selectedScreenshotIndex + 1}`}
              decoding="async"
              ref={(element) => {
                if (element && element.complete && element.naturalWidth > 0) {
                  rememberDims(activeFullSrc, element);
                  markFullLoaded(activeFullSrc);
                }
              }}
              onLoad={(event) => {
                rememberDims(activeFullSrc, event.currentTarget);
                markFullLoaded(activeFullSrc);
              }}
              onError={() => markFullFailed(activeFullSrc)}
            />

            {stageState === 'is-loading' && <div className="info-lightbox-spinner" aria-label="Loading screenshot" />}
            {stageState === 'is-error' && <div className="info-lightbox-error">Full-size image unavailable</div>}

            {screenshotCount > 1 && (
              <div className="info-lightbox-counter">
                {selectedScreenshotIndex + 1} / {screenshotCount}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};
