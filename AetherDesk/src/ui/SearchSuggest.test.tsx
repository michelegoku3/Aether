import { describe, expect, it } from 'vitest';
import { MAX_SUGGEST_ITEMS, limitSuggestItems } from './SearchSuggest';

const numbers = (count: number): number[] => Array.from({ length: count }, (_, index) => index + 1);

describe('limitSuggestItems', () => {
  it('keeps the incoming order', () => {
    const items = [5, 3, 9, 1];

    expect(limitSuggestItems(items)).toEqual([5, 3, 9, 1]);
  });

  it('truncates to the cap, keeping the highest ranked items', () => {
    const items = numbers(50);

    expect(limitSuggestItems(items)).toEqual(numbers(MAX_SUGGEST_ITEMS));
  });

  it('returns the same content when the list is under the cap', () => {
    const items = numbers(3);

    expect(limitSuggestItems(items)).toEqual(items);
  });

  it('honors an explicit max', () => {
    expect(limitSuggestItems(numbers(10), 2)).toEqual([1, 2]);
  });

  it('handles an empty list', () => {
    expect(limitSuggestItems([])).toEqual([]);
  });

  it('does not mutate the input list', () => {
    const items = numbers(30);

    limitSuggestItems(items);

    expect(items).toHaveLength(30);
  });
});
