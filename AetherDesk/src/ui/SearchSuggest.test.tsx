import { fireEvent, render, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { INITIAL_SUGGEST_ITEMS, SUGGEST_ITEMS_STEP, SearchSuggest, type SearchSuggestItem } from './SearchSuggest';

const items = (count: number): SearchSuggestItem[] =>
  Array.from({ length: count }, (_, index) => ({
    id: index + 1,
    name: `Game ${index + 1}`,
    appId: String(1000 + index),
  }));

const renderSuggest = (count: number, activeIndex: number | null = null, resetKey = 'query') =>
  render(
    <SearchSuggest
      open
      items={items(count)}
      emptyText="No results."
      activeIndex={activeIndex}
      resetKey={resetKey}
      onHoverIndex={vi.fn()}
      onSelect={vi.fn()}
    />
  );

const resultRows = (container: HTMLElement) => container.querySelectorAll('.home-search-result').length;

/** jsdom has no layout, so the scroll metrics are stubbed on the list. */
const scrollToBottom = (container: HTMLElement) => {
  const list = container.querySelector<HTMLElement>('.home-search-results');
  if (!list) throw new Error('results list missing');
  Object.defineProperty(list, 'scrollHeight', { configurable: true, value: 2000 });
  Object.defineProperty(list, 'clientHeight', { configurable: true, value: 200 });
  Object.defineProperty(list, 'scrollTop', { configurable: true, value: 1900, writable: true });
  fireEvent.scroll(list);
};

describe('SearchSuggest windowing', () => {
  it('renders a first window and appends the rest while scrolling', () => {
    const { container } = renderSuggest(60);
    expect(resultRows(container)).toBe(INITIAL_SUGGEST_ITEMS);

    scrollToBottom(container);
    expect(resultRows(container)).toBe(INITIAL_SUGGEST_ITEMS + SUGGEST_ITEMS_STEP);

    scrollToBottom(container);
    expect(resultRows(container)).toBe(60);
  });

  it('grows the window to keep the keyboard-selected row mounted', async () => {
    const { container } = renderSuggest(60, 25);

    await waitFor(() => expect(resultRows(container)).toBeGreaterThanOrEqual(26));
  });

  it('resets the window when the query changes', async () => {
    const { container, rerender } = renderSuggest(60);
    scrollToBottom(container);
    expect(resultRows(container)).toBe(INITIAL_SUGGEST_ITEMS + SUGGEST_ITEMS_STEP);

    rerender(
      <SearchSuggest
        open
        items={items(60)}
        emptyText="No results."
        activeIndex={null}
        resetKey="next"
        onHoverIndex={vi.fn()}
        onSelect={vi.fn()}
      />
    );

    await waitFor(() => expect(resultRows(container)).toBe(INITIAL_SUGGEST_ITEMS));
  });
});
