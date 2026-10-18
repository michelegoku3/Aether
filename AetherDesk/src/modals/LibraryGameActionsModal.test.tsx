import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { LibraryGameActionsModal } from './LibraryGameActionsModal';
import type { InstalledGame } from '../hooks/useLibraryGames';

const { useLibraryGamesMock, queryGameStateMock } = vi.hoisted(() => ({
  useLibraryGamesMock: vi.fn(),
  queryGameStateMock: vi.fn(),
}));

vi.mock('../hooks/useLibraryGames', () => ({ useLibraryGames: useLibraryGamesMock }));
vi.mock('../ui/GameHeroImage', () => ({ GameHeroImage: () => null }));

const game = (id: number, state: Partial<InstalledGame> = {}): InstalledGame => ({
  id,
  name: `Game ${id}`,
  appId: String(id),
  installDir: '',
  libraryPath: '',
  gamePath: '',
  installed: false,
  installing: false,
  ...state,
});

const renderModal = (snapshot: InstalledGame, live: InstalledGame[]) => {
  useLibraryGamesMock.mockReturnValue({
    games: live,
    isLoading: false,
    isRefreshing: false,
    status: { text: '', type: 'info' },
    setStatus: vi.fn(),
    loadInstalledGames: vi.fn(),
    queryGameState: queryGameStateMock,
    invalidateGameState: vi.fn(),
  });
  render(
    <LibraryGameActionsModal
      game={snapshot}
      isProcessing={false}
      onClose={vi.fn()}
      onStatus={vi.fn()}
      onRefresh={vi.fn()}
      onOpenVersionEditor={vi.fn()}
    />
  );
};

const onlineButton = () => screen.getByRole('button', { name: 'ONLINE' });
const removeButton = () => screen.getByRole('button', { name: 'Remove' });

describe('LibraryGameActionsModal install state', () => {
  beforeEach(() => {
    queryGameStateMock.mockReset();
  });

  it('enables ONLINE from the live list when Steam finished after the card was opened', () => {
    // The card was clicked while the game was still downloading.
    renderModal(game(100), [game(100, { installed: true, gamePath: 'C:\\Steam\\Game100' })]);

    expect(onlineButton()).toBeEnabled();
    expect(removeButton()).toBeDisabled();
  });

  it('keeps ONLINE and Remove unavailable while Steam is still installing', () => {
    renderModal(game(100, { installing: true }), [game(100, { installing: true })]);

    expect(onlineButton()).toBeDisabled();
    expect(removeButton()).toBeDisabled();
  });

  it('offers Remove only for a game Steam does not hold at all', () => {
    renderModal(game(100), [game(100)]);

    expect(onlineButton()).toBeDisabled();
    expect(removeButton()).toBeEnabled();
  });
});
