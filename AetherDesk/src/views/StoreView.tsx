import { memo, useMemo, useState } from 'react';
import ChangeVersionModal from '../modals/ChangeVersionModal';
import { GameInfoModal } from '../modals/GameInfoModal';
import { LocalDownloadModal } from '../modals/LocalDownloadModal';
import type { StoreSortMode } from '../constants/store';
import { useOptionalLibraryGames } from '../hooks/useLibraryGames';
import type { StoreGameResult } from '../hooks/useStoreSearch';
import type { GameCardAction } from '../ui/GameCard';
import { FolderPlusIcon } from '../ui/icons';
import { StatusAlert } from '../ui/StatusAlert';
import { StoreDownloadModal } from './store/StoreDownloadModal';
import { StoreGameGrid } from './store/StoreGameGrid';
import { StoreSearchBar } from './store/StoreSearchBar';
import { useStoreCatalog } from './store/useStoreCatalog';
import { useStoreDownload } from './store/useStoreDownload';

interface StoreViewProps {
  /** Hidden views stay mounted; only the visible tab may start network work. */
  active: boolean;
  onRefreshUsage?: (forcedKey?: string) => Promise<void>;
  settingsRevision: number;
  settingsReady: boolean;
  useAlternativeGameCards: boolean;
  alternativeCardsOpacity: number;
  alternativeCardsFade: number;
}

export const StoreView = memo(function StoreView({
  active,
  onRefreshUsage,
  settingsRevision,
  settingsReady,
  useAlternativeGameCards,
  alternativeCardsOpacity,
  alternativeCardsFade,
}: StoreViewProps) {
  const library = useOptionalLibraryGames();
  const catalog = useStoreCatalog({
    active,
    settingsRevision,
    settingsReady,
    libraryGames: library?.games,
  });
  const download = useStoreDownload({ onRefreshUsage });
  const [infoGame, setInfoGame] = useState<StoreGameResult | null>(null);
  const [showBulkLocalImport, setShowBulkLocalImport] = useState(false);

  const cardActions = useMemo<Array<GameCardAction<StoreGameResult>>>(
    () => [
      { label: 'Download', variant: 'primary', onClick: download.openDownload },
      { label: 'Info', variant: 'secondary', onClick: setInfoGame },
    ],
    [download.openDownload]
  );

  return (
    <div className="store-view" ref={catalog.scrollRef}>
      <div className="store-header">
        <h1 className="store-title">Store</h1>
        <p className="store-subtitle">Browse, search and unlock game manifests using AetherDesk's built-in database.</p>
      </div>
      <div className="store-separator"></div>

      <StoreSearchBar isLoading={catalog.isLoading} onSearch={catalog.runSearch} onClear={catalog.clearSearch} />
      <div className="store-separator"></div>

      {catalog.searchError && <StatusAlert status={{ text: `Search failed: ${catalog.searchError}`, type: 'error' }} />}
      <div style={{ display: 'flex', gap: '12px', padding: '8px 0', flexWrap: 'wrap', alignItems: 'center' }}>
        <button
          className="settings-small-btn"
          onClick={() => catalog.refreshCatalog()}
          disabled={catalog.isTrendingLoading}
        >
          {catalog.isTrendingLoading ? 'Refreshing…' : 'Refresh'}
        </button>
        <label style={{ fontSize: '13px' }}>
          View:{' '}
          <select
            className="settings-select"
            value={catalog.storeFrontFilter}
            onChange={(e) => {
              catalog.setStoreFrontFilter(e.target.value);
              catalog.setPage(1);
            }}
            style={{ width: '140px' }}
          >
            <option value="trending">Trending</option>
            <option value="top_sellers">Top Sellers</option>
            <option value="latest">New Releases</option>
            <option value="popular_upcoming">Popular Upcoming</option>
            <option value="upcoming">Upcoming</option>
            <option value="discounts">Specials</option>
          </select>
        </label>
        <label style={{ fontSize: '13px' }}>
          Sort:{' '}
          <select
            className="settings-select"
            value={catalog.sortMode}
            onChange={(e) => {
              catalog.setSortMode(e.target.value as StoreSortMode);
              catalog.setPage(1);
            }}
            style={{ width: '140px' }}
          >
            <option value="newest">Newest</option>
            <option value="trending">Trending</option>
            <option value="name">Name A-Z</option>
            <option value="discount">Biggest Discount</option>
            <option value="price_asc">Price: Low–High</option>
          </select>
        </label>
        <label style={{ fontSize: '13px' }}>
          Genre:{' '}
          <select
            className="settings-select"
            value={catalog.genreFilter}
            onChange={(e) => {
              catalog.setGenreFilter(e.target.value);
              catalog.setPage(1);
            }}
            style={{ width: '160px' }}
          >
            <option value="">All genres</option>
            {catalog.availableGenres.map((g) => (
              <option key={g} value={g}>
                {g}
              </option>
            ))}
          </select>
        </label>
      </div>
      <div className="store-separator"></div>

      <div className="library-badge-filters" role="group" aria-label="Filter store by badge">
        {catalog.STORE_BADGE_FILTERS.map((badge) => (
          <button
            key={badge.id}
            type="button"
            className={`library-badge-filter${catalog.badgeFilter === badge.id ? ' active' : ''}`}
            aria-pressed={catalog.badgeFilter === badge.id}
            onClick={() => catalog.setBadgeFilter(badge.id)}
          >
            {badge.label}
          </button>
        ))}
        <button
          type="button"
          className={`library-badge-filter${catalog.hideInLibrary ? ' active' : ''}`}
          aria-pressed={catalog.hideInLibrary}
          onClick={catalog.toggleHideInLibrary}
        >
          Hide in Library
        </button>
        {catalog.badgeFilter !== 'all' && (
          <span className="store-filter-hint">Filtering loaded results (badges load per page).</span>
        )}
      </div>

      <div className="store-separator"></div>

      <StoreGameGrid
        games={catalog.pageGames}
        actions={cardActions}
        isLoading={catalog.isLoading}
        isTrendingLoading={catalog.isTrendingLoading}
        hasSearched={catalog.hasSearched}
        searchError={catalog.searchError}
        activeQuery={catalog.activeQuery}
        page={catalog.page}
        totalPages={catalog.totalPages}
        resultsLength={catalog.resultsLength}
        useAlternativeGameCards={useAlternativeGameCards}
        alternativeCardsOpacity={alternativeCardsOpacity}
        alternativeCardsFade={alternativeCardsFade}
        onPageChange={catalog.setPage}
      />

      {infoGame && (
        <GameInfoModal
          appId={Number(infoGame.appId)}
          fallbackName={infoGame.name}
          fallbackImageUrl={infoGame.imageUrl}
          onClose={() => setInfoGame(null)}
        />
      )}

      {download.selectedGame && (
        <StoreDownloadModal
          game={download.selectedGame}
          source={download.source}
          status={download.status}
          isDownloading={download.isDownloading}
          onSourceChange={download.setSource}
          onDownloadLatest={() => void download.downloadLatest()}
          onDownloadSpecific={() => void download.downloadSpecific()}
          onClose={download.closeDownload}
        />
      )}

      {download.versionGame && (
        <ChangeVersionModal
          game={download.versionGame}
          initialRows={download.manifestRows}
          onClose={download.closeVersion}
        />
      )}

      <button
        type="button"
        className="store-local-fab"
        onClick={() => setShowBulkLocalImport(true)}
        title="Bulk Local import"
        aria-label="Bulk Local Import"
      >
        <FolderPlusIcon size={20} />
      </button>

      {showBulkLocalImport && (
        <LocalDownloadModal
          onClose={() => setShowBulkLocalImport(false)}
          onInstalled={() => {
            onRefreshUsage?.();
          }}
        />
      )}
    </div>
  );
});
