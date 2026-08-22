import { ReactNode, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { GameCover } from '../ui/GameCover';

interface GameInfoPrice {
  currency?: string | null;
  initialCents?: number | null;
  finalCents?: number | null;
  formattedFinal?: string | null;
  discountPercent?: number | null;
}

interface GameInfoPlatforms {
  windows?: boolean | null;
  mac?: boolean | null;
  linux?: boolean | null;
  steamDeckCompatCategory?: number | null;
  steamOsCompatCategory?: number | null;
  steamMachineCompatCategory?: number | null;
  hasVrSupport?: boolean | null;
}

interface GameInfoStoreCategories {
  supportedPlayerCategoryIds?: number[];
  featureCategoryIds?: number[];
  controllerCategoryIds?: number[];
}

interface GameInfoScreenshot {
  id?: number | null;
  thumbnail?: string | null;
  full?: string | null;
  pathThumbnail?: string | null;
  pathFull?: string | null;
  path_thumbnail?: string | null;
  path_full?: string | null;
}

interface GameInfoAppDetails {
  requiredAge?: string | null;
  isFree?: boolean | null;
  shortDescription?: string | null;
  supportedLanguages?: string | null;
  website?: string | null;
  headerImage?: string | null;
  capsuleImage?: string | null;
  background?: string | null;
  developers?: string[];
  publishers?: string[];
  genres?: string[];
  categories?: string[];
  recommendationsTotal?: number | null;
  achievementsTotal?: number | null;
  metacriticScore?: number | null;
  releaseDateText?: string | null;
  comingSoon?: boolean | null;
  drmNotice?: string | null;
  screenshots?: GameInfoScreenshot[];
}

interface GameInfoLocal {
  installed: boolean;
  installDir?: string | null;
  libraryPath?: string | null;
  gamePath?: string | null;
  luaInstalled: boolean;
  manifestPinCount: number;
  updatesEnabled?: boolean | null;
}

interface GameInfo {
  appId: number;
  name?: string | null;
  imageUrl?: string | null;
  storeUrl?: string | null;
  storeUrlPath?: string | null;
  kind?: string | null;
  hasManifest?: boolean | null;
  hasDenuvo?: boolean | null;
  hasNsfw?: boolean | null;
  hasDelisted?: boolean | null;
  releaseDateUnix?: number | null;
  originalReleaseDateUnix?: number | null;
  price?: GameInfoPrice | null;
  metascore?: string | null;
  controllerSupport?: string | null;
  platforms?: GameInfoPlatforms | null;
  storeCategories?: GameInfoStoreCategories | null;
  contentDescriptorIds?: number[];
  screenshots?: GameInfoScreenshot[];
  appDetails?: GameInfoAppDetails | null;
  local?: GameInfoLocal | null;
  updatedAtUnix?: number;
  storeSearchUpdatedAtUnix?: number | null;
  storeItemsUpdatedAtUnix?: number | null;
  appdetailsUpdatedAtUnix?: number | null;
  hubcapUpdatedAtUnix?: number | null;
  localUpdatedAtUnix?: number | null;
}

interface GameInfoModalProps {
  appId: number;
  fallbackName: string;
  fallbackImageUrl?: string;
  onClose: () => void;
}

const NA = 'N/A';

const CONTENT_DESCRIPTOR_LABELS: Record<number, string> = {
  1: 'Some Nudity or Sexual Content',
  2: 'Frequent Violence or Gore',
  3: 'Frequent Nudity or Sexual Content',
  4: 'Adult Only Sexual Content',
  5: 'General Mature Content',
};

const yesNo = (value?: boolean | null) => {
  if (value === true) return 'Yes';
  if (value === false) return 'No';
  return NA;
};

const formatUnixDate = (value?: number | null) => {
  if (!value) return NA;
  return new Date(value * 1000).toLocaleDateString();
};

const formatList = (items?: string[]) => {
  const clean = (items || []).map((item) => item.trim()).filter(Boolean);
  return clean.length > 0 ? clean.join(', ') : NA;
};

const platformSummary = (platforms?: GameInfoPlatforms | null) => {
  if (!platforms) return NA;
  const items = [
    platforms.windows ? 'Windows' : null,
    platforms.mac ? 'macOS' : null,
    platforms.linux ? 'Linux' : null,
    platforms.hasVrSupport ? 'VR' : null,
  ].filter(Boolean);
  return items.length > 0 ? items.join(', ') : NA;
};

const priceSummary = (price?: GameInfoPrice | null, isFree?: boolean | null) => {
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

const cacheAge = (timestamp?: number | null) => {
  if (!timestamp) return NA;
  const seconds = Math.max(0, Math.floor(Date.now() / 1000) - timestamp);
  if (seconds < 60) return 'Just now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ago`;
  return `${Math.floor(seconds / 86400)} d ago`;
};

const displayKind = (kind?: string | null) => {
  const value = kind?.trim();
  if (!value) return NA;
  return value
    .replace(/[_-]+/g, ' ')
    .split(/\s+/)
    .map((word) => (word.toLowerCase() === 'dlc' ? 'DLC' : word.charAt(0).toUpperCase() + word.slice(1).toLowerCase()))
    .join(' ');
};

const contentDescriptorSummary = (ids?: number[]) => {
  const labels = (ids || []).map((id) => CONTENT_DESCRIPTOR_LABELS[id] || `Descriptor ${id}`);
  return labels.length > 0 ? labels.join(', ') : NA;
};

const screenshotThumbnail = (shot: GameInfoScreenshot) =>
  shot.thumbnail || shot.pathThumbnail || shot.path_thumbnail || shot.full || shot.pathFull || shot.path_full || '';

const screenshotFull = (shot: GameInfoScreenshot) =>
  shot.full || shot.pathFull || shot.path_full || shot.thumbnail || shot.pathThumbnail || shot.path_thumbnail || '';

const decodeHtmlEntities = (value?: string | null) => {
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

const normalizeScreenshots = (...groups: Array<GameInfoScreenshot[] | undefined>) => {
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

const InfoRow = ({ label, value, clamp = false }: { label: string; value?: string | number | null; clamp?: boolean }) => (
  <div className="info-row">
    <span className="info-label">{label}</span>
    <span className={`info-value${clamp ? ' info-value-clamp' : ''}`}>
      {value === undefined || value === null || value === '' ? NA : value}
    </span>
  </div>
);

const InfoSection = ({ title, children, className = '' }: { title: string; children: ReactNode; className?: string }) => (
  <section className={`info-section ${className}`.trim()}>
    <h3 className="info-section-title">{title}</h3>
    <div className="info-section-body">{children}</div>
  </section>
);

export const GameInfoModal = ({ appId, fallbackName, fallbackImageUrl, onClose }: GameInfoModalProps) => {
  const [info, setInfo] = useState<GameInfo | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState('');
  const [selectedScreenshotIndex, setSelectedScreenshotIndex] = useState<number | null>(null);
  const [isEdgeZone, setIsEdgeZone] = useState(false);

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
  const screenshots = normalizeScreenshots(info?.screenshots, details?.screenshots);
  const shortDescription = decodeHtmlEntities(details?.shortDescription);

  const handlePrevScreenshot = (e?: React.MouseEvent) => {
    e?.stopPropagation();
    if (screenshots.length === 0) return;
    setSelectedScreenshotIndex((prev) =>
      prev !== null ? (prev - 1 + screenshots.length) % screenshots.length : 0,
    );
  };

  const handleNextScreenshot = (e?: React.MouseEvent) => {
    e?.stopPropagation();
    if (screenshots.length === 0) return;
    setSelectedScreenshotIndex((prev) =>
      prev !== null ? (prev + 1) % screenshots.length : 0,
    );
  };

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
  }, [selectedScreenshotIndex, screenshots.length]);

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
                            <img src={src} alt={alt} />
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

      {selectedScreenshotIndex !== null && screenshots[selectedScreenshotIndex] && (
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
          <div className="info-lightbox-content" onClick={(event) => event.stopPropagation()}>
            <button
              type="button"
              className="info-lightbox-close"
              onClick={() => setSelectedScreenshotIndex(null)}
              aria-label="Close screenshot preview"
            >
              &times;
            </button>

            {screenshots.length > 1 && (
              <>
                <button
                  type="button"
                  className={`info-lightbox-nav prev ${isEdgeZone ? 'visible' : ''}`}
                  onClick={handlePrevScreenshot}
                  aria-label="Previous screenshot"
                  title="Previous screenshot"
                >
                  <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
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
                  <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                    <polyline points="9 18 15 12 9 6" />
                  </svg>
                </button>
              </>
            )}

            <img
              src={screenshots[selectedScreenshotIndex].full || screenshots[selectedScreenshotIndex].thumbnail || ''}
              alt={`${title} screenshot ${selectedScreenshotIndex + 1}`}
            />
          </div>
        </div>
      )}
    </div>
  );
};
