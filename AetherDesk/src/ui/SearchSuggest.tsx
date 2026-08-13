import { useEffect, useRef } from 'react';

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
  onHoverIndex: (index: number) => void;
  onSelect: (item: SearchSuggestItem) => void;
}

const ROW_HEIGHT = 42;

/** Shared typeahead list (Home library + Store Steam suggest). */
export const SearchSuggest = ({
  open,
  items,
  emptyText,
  statusText,
  activeIndex,
  maxVisible = 5,
  onHoverIndex,
  onSelect,
}: SearchSuggestProps) => {
  const resultRefs = useRef<Array<HTMLButtonElement | null>>([]);

  useEffect(() => {
    if (!open || activeIndex === null) return;
    resultRefs.current[activeIndex]?.scrollIntoView({ block: 'nearest' });
  }, [activeIndex, open]);

  if (!open) return null;

  const extraStatus = statusText ? 1 : 0;
  const visibleRowCount = Math.min(maxVisible, Math.max(items.length + extraStatus, 1));

  return (
    <div
      className="home-search-results"
      style={{ maxHeight: `${visibleRowCount * ROW_HEIGHT}px` }}
      onWheel={(event) => event.stopPropagation()}
    >
      {items.length > 0 ? (
        items.map((item, index) => (
          <button
            key={`${item.appId}-${item.id}`}
            type="button"
            ref={(element) => { resultRefs.current[index] = element; }}
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
      {items.length > 0 && statusText ? (
        <div className="home-search-empty">{statusText}</div>
      ) : null}
    </div>
  );
};

export const moveSuggestIndex = (
  current: number | null,
  itemCount: number,
  direction: 1 | -1,
): number | null => {
  if (itemCount === 0) return null;
  if (current === null) {
    return direction === 1 ? 0 : itemCount - 1;
  }
  if (direction === 1) {
    return Math.min(current + 1, itemCount - 1);
  }
  return Math.max(current - 1, 0);
};
