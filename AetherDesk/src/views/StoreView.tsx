import { memo, useMemo, useState } from 'react';
import ChangeVersionModal from '../modals/ChangeVersionModal';
import { GameInfoModal } from '../modals/GameInfoModal';
import { LocalDownloadModal } from '../modals/LocalDownloadModal';
import type { StoreGameResult } from '../hooks/useStoreSearch';
import type { GameCardAction } from '../ui/GameCard';
import { FolderPlusIcon } from '../ui/icons';
import { StoreDownloadModal } from './store/StoreDownloadModal';
import { StoreGameGrid } from './store/StoreGameGrid';
import { StoreSearchBar } from './store/StoreSearchBar';
import { useStoreCatalog } from './store/useStoreCatalog';
import { useStoreDownload } from './store/useStoreDownload';

interface StoreViewProps {
  onRefreshUsage?: (forcedKey?: string) => Promise<void>;
  settingsRevision: number;
  /** Initial settings hydration completed in App; safe to start background preload. */
  settingsReady: boolean;
  useAlternativeGameCards: boolean;
  alternativeCardsOpacity: number;
  alternativeCardsFade: number;
}

export const StoreView = memo(function StoreView({
  onRefreshUsage,
  settingsRevision,
  settingsReady,
  useAlternativeGameCards,
  alternativeCardsOpacity,
  alternativeCardsFade,
}: StoreViewProps) {
  const catalog = useStoreCatalog({ settingsRevision, settingsReady });
  const download = useStoreDownload({ onRefreshUsage });
  const [infoGame, setInfoGame] = useState<StoreGameResult | null>(null);
  const [showBulkLocalImport, setShowBulkLocalImport] = useState(false);

  const cardActions = useMemo<Array<GameCardAction<StoreGameResult>>>(() => [
    { label: 'Download', variant: 'primary', onClick: download.openDownload },
    { label: 'Info', variant: 'secondary', onClick: setInfoGame },
  ], [download.openDownload]);

  return (
    <div className="store-view" ref={catalog.scrollRef}>
      <div className="store-header">
        <h1 className="store-title">Store</h1>
        <p className="store-subtitle">Browse, search and unlock game manifests using AetherDesk's built-in database.</p>
      </div>
      <div className="store-separator"></div>

      <StoreSearchBar
        isLoading={catalog.isLoading}
        onSearch={catalog.runSearch}
        onClear={catalog.clearSearch}
      />
      <div className="store-separator"></div>

      <StoreGameGrid
        games={catalog.pageGames}
        actions={cardActions}
        isLoading={catalog.isLoading}
        isTrendingLoading={catalog.isTrendingLoading}
        hasSearched={catalog.hasSearched}
        activeQuery={catalog.activeQuery}
        page={catalog.page}
        totalPages={catalog.totalPages}
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
        title="Bulk Local Import (.lua, .manifest, archives)"
        aria-label="Bulk Local Import"
      >
        <FolderPlusIcon size={20} />
      </button>

      {showBulkLocalImport && (
        <LocalDownloadModal
          onClose={() => setShowBulkLocalImport(false)}
          onInstalled={() => { onRefreshUsage?.(); }}
        />
      )}
    </div>
  );
});
