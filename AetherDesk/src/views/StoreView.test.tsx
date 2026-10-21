import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { StoreView } from './StoreView';

const { catalog, download } = vi.hoisted(() => ({
  catalog: {
    scrollRef: { current: null },
    page: 1,
    setPage: vi.fn(),
    pageGames: [],
    totalPages: 1,
    isLoading: false,
    isTrendingLoading: false,
    hasSearched: false,
    activeQuery: '',
    searchError: null as string | null,
    runSearch: vi.fn().mockResolvedValue(undefined),
    clearSearch: vi.fn(),
  },
  download: {
    selectedGame: null,
    source: 'hubcap',
    status: { text: '', type: 'info' as const },
    isDownloading: false,
    versionGame: null,
    manifestRows: [],
    setSource: vi.fn(),
    openDownload: vi.fn(),
    closeDownload: vi.fn(),
    downloadLatest: vi.fn(),
    downloadSpecific: vi.fn(),
    closeVersion: vi.fn(),
  },
}));

vi.mock('./store/useStoreCatalog', () => ({ useStoreCatalog: () => catalog }));
vi.mock('./store/useStoreDownload', () => ({ useStoreDownload: () => download }));
vi.mock('./store/StoreDownloadModal', () => ({ StoreDownloadModal: () => null }));
vi.mock('../modals/ChangeVersionModal', () => ({ default: () => null }));
vi.mock('../modals/GameInfoModal', () => ({ GameInfoModal: () => null }));
vi.mock('../modals/LocalDownloadModal', () => ({ default: () => null }));

const renderView = () =>
  render(
    <StoreView
      settingsRevision={1}
      settingsReady
      useAlternativeGameCards={false}
      alternativeCardsOpacity={100}
      alternativeCardsFade={0}
    />
  );

describe('StoreView', () => {
  it('reports a failed search and does not claim an empty catalog', () => {
    catalog.searchError = 'provider timeout';
    catalog.hasSearched = true;
    catalog.activeQuery = 'doom';
    renderView();

    expect(screen.getByText(/provider timeout/)).toBeInTheDocument();
    expect(screen.queryByText(/No games found/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Enter a query above/)).not.toBeInTheDocument();
  });

  it('shows the empty state when no search failed', () => {
    catalog.searchError = null;
    catalog.hasSearched = true;
    catalog.activeQuery = 'doom';
    renderView();

    expect(screen.getByText(/No games found for "doom"/)).toBeInTheDocument();
    expect(screen.queryByText(/Search failed/)).not.toBeInTheDocument();
  });
});
