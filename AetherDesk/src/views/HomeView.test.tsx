import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { HomeView } from './HomeView';
import { mockInvokeCommands } from '../test/tauriMocks';
import { INITIAL_SUGGEST_ITEMS, SUGGEST_ITEMS_STEP } from '../ui/SearchSuggest';
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

describe('HomeView suggestion window', () => {
  // A whole library is what an empty query offers on focus.
  const libraryGames = Array.from({ length: 50 }, (_, index) => game(index + 1));

  beforeEach(() => {
    mockInvokeCommands({ get_antivirus_exclusion_done: true });
    useLibraryGamesMock.mockReturnValue({ games: libraryGames, isLoading: false });
  });

  const suggestionButtons = (container: HTMLElement) =>
    Array.from(container.querySelectorAll<HTMLButtonElement>('.home-search-result'));

  /** jsdom has no layout, so the scroll metrics are stubbed on the list. */
  const scrollToBottom = (container: HTMLElement) => {
    const list = container.querySelector<HTMLElement>('.home-search-results');
    if (!list) throw new Error('results list missing');
    Object.defineProperty(list, 'scrollHeight', { configurable: true, value: 2000 });
    Object.defineProperty(list, 'clientHeight', { configurable: true, value: 200 });
    Object.defineProperty(list, 'scrollTop', { configurable: true, value: 1900, writable: true });
    fireEvent.scroll(list);
  };

  it('renders a first window and appends more while scrolling', async () => {
    const { container } = render(<HomeView />);
    const user = userEvent.setup();

    await user.click(screen.getByPlaceholderText('Search a Lua game...'));

    expect(suggestionButtons(container)).toHaveLength(INITIAL_SUGGEST_ITEMS);

    scrollToBottom(container);
    expect(suggestionButtons(container)).toHaveLength(INITIAL_SUGGEST_ITEMS + SUGGEST_ITEMS_STEP);
  });

  it('lets keyboard navigation reach a game beyond the first window', async () => {
    const { container } = render(<HomeView />);
    const user = userEvent.setup();
    const input = screen.getByPlaceholderText('Search a Lua game...');

    // A query every game matches keeps the list long.
    await user.type(input, 'Game');
    await user.keyboard('{ArrowDown}'.repeat(libraryGames.length + 5));

    const suggestions = suggestionButtons(container);
    expect(suggestions).toHaveLength(libraryGames.length);
    expect(suggestions[suggestions.length - 1]).toHaveClass('active');

    await user.keyboard('{Enter}');
    expect(input).toHaveValue('Game 50');
  });
});
