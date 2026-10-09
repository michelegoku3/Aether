import { useEffect, useRef, useState, type FormEvent } from 'react';
import { SearchSuggest, moveSuggestIndex } from '../../ui/SearchSuggest';
import { useSteamSuggest } from '../../hooks/useSteamSuggest';

interface StoreSearchBarProps {
  isLoading: boolean;
  onSearch: (query: string) => Promise<void>;
  onClear: () => void;
}

export function StoreSearchBar({ isLoading, onSearch, onClear }: StoreSearchBarProps) {
  const [query, setQuery] = useState('');
  const [isSuggestOpen, setIsSuggestOpen] = useState(false);
  const [activeSuggestIndex, setActiveSuggestIndex] = useState<number | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const { items, isLoading: isSuggestLoading } = useSteamSuggest(query, isSuggestOpen);

  useEffect(() => {
    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (target && !panelRef.current?.contains(target)) setIsSuggestOpen(false);
    };
    document.addEventListener('mousedown', handlePointerDown);
    return () => document.removeEventListener('mousedown', handlePointerDown);
  }, []);

  useEffect(() => setActiveSuggestIndex(null), [query]);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (isSuggestOpen && activeSuggestIndex !== null && items[activeSuggestIndex]) {
      const selected = items[activeSuggestIndex].name;
      setQuery(selected);
      setIsSuggestOpen(false);
      setActiveSuggestIndex(null);
      await onSearch(selected);
      return;
    }
    setIsSuggestOpen(false);
    setActiveSuggestIndex(null);
    await onSearch(query);
  };

  const selectSuggestion = (name: string) => {
    setQuery(name);
    setIsSuggestOpen(false);
    setActiveSuggestIndex(null);
    void onSearch(name);
  };

  return (
    <form onSubmit={(event) => void submit(event)} className="store-search-form">
      <div className="home-search-wrapper store-suggest-wrap" ref={panelRef}>
        <input
          type="text"
          placeholder="Search games by name or App ID on Steam..."
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setIsSuggestOpen(true);
            setActiveSuggestIndex(null);
          }}
          onFocus={() => setIsSuggestOpen(true)}
          onKeyDown={(event) => {
            if (event.key === 'ArrowDown') {
              event.preventDefault();
              setIsSuggestOpen(true);
              setActiveSuggestIndex((current) => moveSuggestIndex(current, items.length, 1));
            } else if (event.key === 'ArrowUp') {
              event.preventDefault();
              setIsSuggestOpen(true);
              setActiveSuggestIndex((current) => moveSuggestIndex(current, items.length, -1));
            } else if (event.key === 'Escape') {
              setIsSuggestOpen(false);
            }
          }}
          className="store-search-input"
        />
        {query && (
          <button
            type="button"
            className="home-search-clear"
            aria-label="Clear search"
            onClick={() => {
              setQuery('');
              setIsSuggestOpen(false);
              setActiveSuggestIndex(null);
              onClear();
            }}
          >
            &times;
          </button>
        )}
        <SearchSuggest
          open={isSuggestOpen && query.trim().length >= 2}
          items={items}
          emptyText="No Steam suggestions."
          statusText={isSuggestLoading ? 'Searching Steam…' : undefined}
          activeIndex={activeSuggestIndex}
          onHoverIndex={setActiveSuggestIndex}
          onSelect={(item) => selectSuggestion(item.name)}
        />
      </div>
      <button type="submit" className="store-search-btn" disabled={isLoading} title="Search Catalog">
        {isLoading ? (
          <span style={{ fontSize: '12px' }}>...</span>
        ) : (
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" style={{ display: 'block' }}>
            <circle cx="11" cy="11" r="8"></circle>
            <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
          </svg>
        )}
      </button>
    </form>
  );
}
