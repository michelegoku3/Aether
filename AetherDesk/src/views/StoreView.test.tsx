import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invokeMock } from '../test/tauriMocks';
import type { InstalledGame } from '../hooks/useLibraryGames';
import { StoreView } from './StoreView';

const mockLibraryGames = vi.hoisted(() => ({ current: [] as InstalledGame[] }));
vi.mock('../hooks/useLibraryGames', () => ({
  useOptionalLibraryGames: () => ({ games: mockLibraryGames.current }),
}));

vi.mock('./store/useStoreDownload', () => ({
  useStoreDownload: () => ({
    selectedGame: null,
    source: 'hubcap',
    status: { text: '', type: 'info' },
    isDownloading: false,
    versionGame: null,
    manifestRows: [],
    setSource: vi.fn(),
    openDownload: vi.fn(),
    closeDownload: vi.fn(),
    downloadLatest: vi.fn(),
    downloadSpecific: vi.fn(),
    closeVersion: vi.fn(),
  }),
}));
vi.mock('./store/StoreDownloadModal', () => ({ StoreDownloadModal: () => null }));
vi.mock('../modals/ChangeVersionModal', () => ({ default: () => null }));
vi.mock('../modals/GameInfoModal', () => ({ GameInfoModal: () => null }));
vi.mock('../modals/LocalDownloadModal', () => ({ default: () => null }));
// The trending catalog and the suggestion list would add unrelated content.
vi.mock('./store/useTrendingStore', () => ({
  useTrendingStore: () => ({ loadNext: vi.fn(), reset: vi.fn(), isLoading: false }),
}));
vi.mock('../hooks/useSteamSuggest', () => ({ useSteamSuggest: () => ({ items: [], isLoading: false }) }));

const witcher = {
  id: 1,
  appId: '1',
  name: 'The Witcher',
  has_manifest: true,
  has_denuvo: false,
};

const renderView = () =>
  render(
    <StoreView
      active
      settingsRevision={1}
      settingsReady
      useAlternativeGameCards={false}
      alternativeCardsOpacity={100}
      alternativeCardsFade={0}
    />
  );

const search = async (user: ReturnType<typeof userEvent.setup>, query: string) => {
  const input = screen.getByPlaceholderText(/Search games by name/);
  await user.clear(input);
  await user.type(input, `${query}{Enter}`);
};

describe('StoreView', () => {
  beforeEach(() => {
    localStorage.clear();
    mockLibraryGames.current = [];
  });

  it('reports a failed search and drops the cards of the previous query', async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'get_cached_store_search') {
        if (invokeMock.mock.calls.filter(([name]) => name === command).length === 1) {
          return { cacheState: 'fresh', results: [witcher] };
        }
        throw new Error('provider timeout');
      }
      if (command === 'check_denuvo_bulk') return {};
      return [];
    });
    renderView();

    await search(user, 'witcher');
    expect(await screen.findByText('The Witcher')).toBeInTheDocument();

    await search(user, 'doom');

    await waitFor(() => expect(screen.getByText(/Search failed: provider timeout/)).toBeInTheDocument());
    expect(screen.queryByText('The Witcher')).not.toBeInTheDocument();
    expect(screen.queryByText(/No games found/)).not.toBeInTheDocument();
  });

  it('shows the empty state when the search finds nothing', async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'get_cached_store_search') return { cacheState: 'fresh', results: [] };
      if (command === 'check_denuvo_bulk') return {};
      return [];
    });
    renderView();

    await search(user, 'doom');

    expect(await screen.findByText(/No games found for "doom"/)).toBeInTheDocument();
    expect(screen.queryByText(/Search failed/)).not.toBeInTheDocument();
  });

  it('marks library games and hides them when Hide in Library is toggled', async () => {
    const user = userEvent.setup();
    mockLibraryGames.current = [
      {
        id: 1,
        appId: '1',
        name: 'The Witcher',
        installDir: '',
        libraryPath: '',
        gamePath: '',
        installed: false,
        installing: false,
      },
    ];
    const doom = { id: 2, appId: '2', name: 'DOOM', has_manifest: true, has_denuvo: false };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'get_cached_store_search') return { cacheState: 'fresh', results: [witcher, doom] };
      if (command === 'check_denuvo_bulk') return {};
      return [];
    });
    renderView();

    await search(user, 'game');
    expect(await screen.findByText('In Library')).toBeInTheDocument();
    expect(screen.getByText('DOOM')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Hide in Library' }));
    expect(screen.queryByText('The Witcher')).not.toBeInTheDocument();
    expect(screen.getByText('DOOM')).toBeInTheDocument();
  });
});
