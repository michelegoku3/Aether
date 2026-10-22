import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { HomeView } from './HomeView';
import { mockInvokeCommands } from '../test/tauriMocks';
import { MAX_SUGGEST_ITEMS } from '../ui/SearchSuggest';
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

describe('HomeView suggestion list bound', () => {
  // A whole library is what an empty query offers on focus.
  const libraryGames = Array.from({ length: 50 }, (_, index) => game(index + 1));

  beforeEach(() => {
    mockInvokeCommands({ get_antivirus_exclusion_done: true });
    useLibraryGamesMock.mockReturnValue({ games: libraryGames, isLoading: false });
  });

  const suggestionButtons = (container: HTMLElement) =>
    Array.from(container.querySelectorAll<HTMLButtonElement>('.home-search-result'));

  it('renders at most MAX_SUGGEST_ITEMS suggestions for the full library', async () => {
    const { container } = render(<HomeView />);
    const user = userEvent.setup();

    await user.click(screen.getByPlaceholderText('Search a Lua game...'));

    expect(suggestionButtons(container).length).toBe(MAX_SUGGEST_ITEMS);
  });

  it('stops keyboard navigation at the last bounded suggestion', async () => {
    const { container } = render(<HomeView />);
    const user = userEvent.setup();
    const input = screen.getByPlaceholderText('Search a Lua game...');

    // A query every game matches keeps the truncated list full.
    await user.type(input, 'Game');
    await user.keyboard('{ArrowDown}'.repeat(MAX_SUGGEST_ITEMS + 5));

    const suggestions = suggestionButtons(container);
    const lastSuggestion = suggestions[suggestions.length - 1];
    expect(suggestions).toHaveLength(MAX_SUGGEST_ITEMS);
    expect(lastSuggestion).toHaveClass('active');

    // Enter acts on the bounded list only, never on an item that is not shown.
    await user.keyboard('{Enter}');
    expect(input).toHaveValue(lastSuggestion.querySelector('.home-search-result-name')?.textContent);
  });
});
