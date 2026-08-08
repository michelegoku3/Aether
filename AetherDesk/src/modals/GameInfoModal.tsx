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

const yesNo = (value?: boolean | null) => {
  if (value === true) return 'Yes';
  if (value === false) return 'No';
  return 'Unknown';
};

const formatUnixDate = (value?: number | null) => {
  if (!value) return 'Unknown';
  return new Date(value * 1000).toLocaleDateString();
};

const formatList = (items?: string[]) => {
  const clean = (items || []).map((item) => item.trim()).filter(Boolean);
  return clean.length > 0 ? clean.join(', ') : 'Unknown';
};

const platformSummary = (platforms?: GameInfoPlatforms | null) => {
  if (!platforms) return 'Unknown';
  const items = [
    platforms.windows ? 'Windows' : null,
    platforms.mac ? 'macOS' : null,
    platforms.linux ? 'Linux' : null,
    platforms.hasVrSupport ? 'VR' : null,
  ].filter(Boolean);
  return items.length > 0 ? items.join(', ') : 'Unknown';
};

const priceSummary = (price?: GameInfoPrice | null, isFree?: boolean | null) => {
  if (isFree) return 'Free to Play';
  if (!price) return 'Unknown';
  if (price.formattedFinal) return price.formattedFinal;
  if (typeof price.finalCents === 'number') {
    const currency = price.currency || 'EUR';
    return `${currency} ${(price.finalCents / 100).toFixed(2)}`;
  }
  return 'Unknown';
};

const cacheAge = (timestamp?: number | null) => {
  if (!timestamp) return 'Never';
  const seconds = Math.max(0, Math.floor(Date.now() / 1000) - timestamp);
  if (seconds < 60) return 'Just now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ago`;
  return `${Math.floor(seconds / 86400)} d ago`;
};

const InfoRow = ({ label, value }: { label: string; value?: string | number | null }) => (
  <div className="info-row">
    <span className="info-label">{label}</span>
    <span className="info-value">{value === undefined || value === null || value === '' ? 'Unknown' : value}</span>
  </div>
);

const InfoSection = ({ title, children }: { title: string; children: ReactNode }) => (
  <section className="info-section">
    <h3 className="info-section-title">{title}</h3>
    <div className="info-section-body">{children}</div>
  </section>
);

export const GameInfoModal = ({ appId, fallbackName, fallbackImageUrl, onClose }: GameInfoModalProps) => {
  const [info, setInfo] = useState<GameInfo | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState('');

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
                  {details?.shortDescription && <p>{details.shortDescription}</p>}
                </div>
              </div>

              <div className="info-grid">
                <InfoSection title="Store">
                  <InfoRow label="Type" value={info.kind} />
                  <InfoRow label="Price" value={priceSummary(info.price, details?.isFree)} />
                  <InfoRow label="Metascore" value={info.metascore || details?.metacriticScore} />
                  <InfoRow label="Release" value={details?.releaseDateText || formatUnixDate(info.releaseDateUnix)} />
                  <InfoRow label="Original release" value={formatUnixDate(info.originalReleaseDateUnix)} />
                  <InfoRow label="Controller" value={info.controllerSupport} />
                  <InfoRow label="Platforms" value={platformSummary(info.platforms)} />
                </InfoSection>

                <InfoSection title="Availability & flags">
                  <InfoRow label="Manifest available" value={yesNo(info.hasManifest)} />
                  <InfoRow label="Denuvo" value={yesNo(info.hasDenuvo)} />
                  <InfoRow label="NSFW" value={yesNo(info.hasNsfw)} />
                  <InfoRow label="Delisted" value={yesNo(info.hasDelisted)} />
                  <InfoRow label="Content descriptors" value={(info.contentDescriptorIds || []).join(', ') || 'None'} />
                </InfoSection>

                <InfoSection title="Publisher data">
                  <InfoRow label="Developers" value={formatList(details?.developers)} />
                  <InfoRow label="Publishers" value={formatList(details?.publishers)} />
                  <InfoRow label="Genres" value={formatList(details?.genres)} />
                  <InfoRow label="Categories" value={formatList(details?.categories)} />
                  <InfoRow label="Recommendations" value={details?.recommendationsTotal} />
                  <InfoRow label="Achievements" value={details?.achievementsTotal} />
                </InfoSection>

                <InfoSection title="Local library">
                  <InfoRow label="Lua installed" value={yesNo(local?.luaInstalled)} />
                  <InfoRow label="Game installed" value={yesNo(local?.installed)} />
                  <InfoRow label="Manifest pins" value={local?.manifestPinCount} />
                  <InfoRow label="Updates enabled" value={yesNo(local?.updatesEnabled)} />
                  <InfoRow label="Install dir" value={local?.installDir} />
                  <InfoRow label="Library path" value={local?.libraryPath} />
                </InfoSection>
              </div>

              <InfoSection title="Cache freshness">
                {cacheSummary.map(([label, value]) => (
                  <InfoRow key={label} label={label} value={value} />
                ))}
              </InfoSection>
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
};
