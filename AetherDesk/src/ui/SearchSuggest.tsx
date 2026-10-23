import { useEffect, useRef, useState, type UIEvent } from 'react';

export interface SearchSuggestItem {
  id: string | number;
  name: string;
  appId: string;
}

interface SearchSuggestProps {
  open: boolean;
  items: SearchSuggestItem[];
  emptyText: string;
  statusText?: string;
  activeIndex: number | null;
  maxVisible?: number;
  /** Resets the rendered window when the query changes. */
  resetKey?: string | number;
  onHoverIndex: (index: number) => void;
  onSelect: (item: SearchSuggestItem) => void;
}

const ROW_HEIGHT = 42;

/**
 * The first window is small so opening the dropdown on an empty query (the
 * whole library) stays cheap. All items remain reachable: the window grows by
 * one step near the bottom, and keyboard selection pulls its row into view.
 */
export const INITIAL_SUGGEST_ITEMS = 20;
export const SUGGEST_ITEMS_STEP = 20;

/** Shared typeahead list (Home library + Store Steam suggest). */
export const SearchSuggest = ({
  open,
  items,
  emptyText,
  statusText,
  activeIndex,
  maxVisible = 5,
  resetKey,
  onHoverIndex,
  onSelect,
}: SearchSuggestProps) => {
  const resultRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const [renderedCount, setRenderedCount] = useState(INITIAL_SUGGEST_ITEMS);

  useEffect(() => {
    setRenderedCount(INITIAL_SUGGEST_ITEMS);
  }, [resetKey]);

  // Keyboard selection can move past the rendered window; mount the row before
  // the scroll effect below asks the DOM to bring it into view.
  useEffect(() => {
    if (!open || activeIndex === null || activeIndex < renderedCount) return;
    const next = Math.ceil((activeIndex + 1) / SUGGEST_ITEMS_STEP) * SUGGEST_ITEMS_STEP;
    setRenderedCount(Math.min(items.length, next));
  }, [activeIndex, items.length, open, renderedCount]);

  useEffect(() => {
    if (!open || activeIndex === null) return;
    resultRefs.current[activeIndex]?.scrollIntoView({ block: 'nearest' });
  }, [activeIndex, open, renderedCount]);

  if (!open) return null;

  const visibleCount = Math.min(renderedCount, items.length);
  const renderedItems = items.slice(0, visibleCount);
  const extraStatus = statusText ? 1 : 0;
  const visibleRowCount = Math.min(maxVisible, Math.max(items.length + extraStatus, 1));

  const handleScroll = (event: UIEvent<HTMLDivElement>) => {
    const list = event.currentTarget;
    if (list.scrollTop + list.clientHeight < list.scrollHeight - ROW_HEIGHT * 2) return;
    setRenderedCount((count) => Math.min(items.length, count + SUGGEST_ITEMS_STEP));
  };

  return (
    <div
      className="home-search-results"
      style={{ maxHeight: `${visibleRowCount * ROW_HEIGHT}px` }}
      onWheel={(event) => event.stopPropagation()}
      onScroll={handleScroll}
    >
      {items.length > 0 ? (
        renderedItems.map((item, index) => (
          <button
            key={`${item.appId}-${item.id}`}
            type="button"
            ref={(element) => {
              resultRefs.current[index] = element;
            }}
            className={`home-search-result ${index === activeIndex ? 'active' : ''}`}
            onMouseEnter={() => onHoverIndex(index)}
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => onSelect(item)}
          >
            <span className="home-search-result-name">{item.name}</span>
            <span className="home-search-result-appid">{item.appId}</span>
          </button>
        ))
      ) : (
        <div className="home-search-empty">{statusText || emptyText}</div>
      )}
      {items.length > 0 && statusText ? <div className="home-search-empty">{statusText}</div> : null}
    </div>
  );
};

export const moveSuggestIndex = (current: number | null, itemCount: number, direction: 1 | -1): number | null => {
  if (itemCount === 0) return null;
  if (current === null) {
    return direction === 1 ? 0 : itemCount - 1;
  }
  if (direction === 1) {
    return Math.min(current + 1, itemCount - 1);
  }
  return Math.max(current - 1, 0);
};
