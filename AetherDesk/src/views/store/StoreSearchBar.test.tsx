import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { StoreSearchBar } from './StoreSearchBar';

const { suggestions } = vi.hoisted(() => ({ suggestions: [
  { id: 1, appId: '10', name: 'First Game' },
  { id: 2, appId: '20', name: 'Second Game' },
] }));

vi.mock('../../hooks/useSteamSuggest', () => ({
  useSteamSuggest: () => ({ items: suggestions, isLoading: false }),
}));

describe('StoreSearchBar', () => {
  it('submits the typed query', async () => {
    const user = userEvent.setup();
    const onSearch = vi.fn().mockResolvedValue(undefined);
    render(<StoreSearchBar isLoading={false} onSearch={onSearch} onClear={vi.fn()} />);

    await user.type(screen.getByPlaceholderText(/search games/i), 'portal');
    await user.click(screen.getByTitle('Search Catalog'));

    expect(onSearch).toHaveBeenCalledWith('portal');
  });

  it('uses the active keyboard suggestion on Enter', async () => {
    const user = userEvent.setup();
    const onSearch = vi.fn().mockResolvedValue(undefined);
    render(<StoreSearchBar isLoading={false} onSearch={onSearch} onClear={vi.fn()} />);
    const input = screen.getByPlaceholderText(/search games/i);

    await user.type(input, 'fi');
    await user.keyboard('{ArrowDown}{Enter}');

    expect(onSearch).toHaveBeenCalledWith('First Game');
    expect(input).toHaveValue('First Game');
  });

  it('clears query, suggestions and catalog state', async () => {
    const user = userEvent.setup();
    const onClear = vi.fn();
    render(<StoreSearchBar isLoading={false} onSearch={vi.fn()} onClear={onClear} />);
    const input = screen.getByPlaceholderText(/search games/i);
    await user.type(input, 'query');

    await user.click(screen.getByRole('button', { name: /clear search/i }));

    expect(input).toHaveValue('');
    expect(onClear).toHaveBeenCalledOnce();
  });

  it('closes suggestions on Escape', async () => {
    const user = userEvent.setup();
    render(<StoreSearchBar isLoading={false} onSearch={vi.fn()} onClear={vi.fn()} />);
    await user.type(screen.getByPlaceholderText(/search games/i), 'fi');
    expect(screen.getByText('First Game')).toBeInTheDocument();

    await user.keyboard('{Escape}');

    expect(screen.queryByText('First Game')).not.toBeInTheDocument();
  });

  it('disables submit while a search is loading', () => {
    render(<StoreSearchBar isLoading onSearch={vi.fn()} onClear={vi.fn()} />);
    expect(screen.getByTitle('Search Catalog')).toBeDisabled();
  });
});
