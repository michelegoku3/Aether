import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { HomeView } from './HomeView';
import { mockInvokeCommands } from '../test/tauriMocks';
import type { InstalledGame } from '../hooks/useLibraryGames';

const { useLibraryGamesMock } = vi.hoisted(() => ({ useLibraryGamesMock: vi.fn() }));

vi.mock('../hooks/useLibraryGames', () => ({ useLibraryGames: useLibraryGamesMock }));

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

const applyCrackButton = () => screen.getByRole('button', { name: 'Apply Crack' });

/** Type a name and pick the suggestion, exactly like a user would. */
const selectGame = async (name: string) => {
  const user = userEvent.setup();
  await user.type(screen.getByPlaceholderText('Search a Lua game...'), name);
  await user.click(await screen.findByRole('button', { name: new RegExp(name) }));
  return user;
};

describe('HomeView game actions', () => {
  beforeEach(() => {
    mockInvokeCommands({ get_antivirus_exclusion_done: true });
    useLibraryGamesMock.mockReturnValue({
      games: [game(100, { name: 'Alpha' })],
      isLoading: false,
    });
  });

  it('leaves Apply Crack tooltip-free and reports the requirement only on click', async () => {
    render(<HomeView />);

    const user = await selectGame('Alpha');

    expect(applyCrackButton()).toBeEnabled();
    expect(applyCrackButton()).not.toHaveAttribute('title');

    await user.click(applyCrackButton());

    await waitFor(() =>
      expect(screen.getByText('Apply Crack requires the selected game to be installed locally.')).toBeInTheDocument()
    );
  });
});
