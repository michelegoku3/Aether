import type { CSSProperties } from 'react';
import type { StoreGameResult } from '../../hooks/useStoreSearch';
import { GameCard, type GameCardAction } from '../../ui/GameCard';

interface StoreGameGridProps {
  games: StoreGameResult[];
  actions: Array<GameCardAction<StoreGameResult>>;
  isLoading: boolean;
  isTrendingLoading: boolean;
  hasSearched: boolean;
  activeQuery: string;
  page: number;
  totalPages: number;
  useAlternativeGameCards: boolean;
  alternativeCardsOpacity: number;
  alternativeCardsFade: number;
  onPageChange: (page: number) => void;
}

export function StoreGameGrid({
  games,
  actions,
  isLoading,
  isTrendingLoading,
  hasSearched,
  activeQuery,
  page,
  totalPages,
  useAlternativeGameCards,
  alternativeCardsOpacity,
  alternativeCardsFade,
  onPageChange,
}: StoreGameGridProps) {
  const alternativeStyle = useAlternativeGameCards ? {
    '--alt-card-opacity': Math.max(0, Math.min(100, alternativeCardsOpacity)),
    '--alt-card-fade': Math.max(0, Math.min(100, alternativeCardsFade)),
  } as CSSProperties : undefined;

  return (
    <>
      <div
        className={useAlternativeGameCards ? 'store-grid alt-card-grid' : 'store-grid'}
        style={alternativeStyle}
      >
        {isLoading || (isTrendingLoading && games.length === 0) ? (
          <div className="store-no-results">
            {isLoading ? 'Loading results from Steam & Hubcap...' : 'Loading trending Steam games...'}
          </div>
        ) : games.length > 0 ? (
          games.map((game) => (
            <GameCard
              key={game.id}
              game={game}
              cardVariant={useAlternativeGameCards ? 'backdrop' : 'classic'}
              actions={actions}
            />
          ))
        ) : (
          <div className="store-no-results">
            {hasSearched ? `No games found for "${activeQuery}"` : 'Enter a query above to search the Steam catalog.'}
          </div>
        )}
      </div>

      {!isLoading && totalPages > 1 && (
        <div className="store-pagination">
          <button
            disabled={page === 1}
            onClick={() => onPageChange(Math.max(page - 1, 1))}
            className="pagination-btn"
          >
            &larr; Prev
          </button>
          <span className="pagination-info">Page {page} of {totalPages}</span>
          <button
            disabled={page >= totalPages}
            onClick={() => onPageChange(Math.min(page + 1, totalPages))}
            className="pagination-btn"
          >
            Next &rarr;
          </button>
        </div>
      )}
    </>
  );
}
