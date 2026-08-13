import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { SearchSuggestItem } from '../ui/SearchSuggest';

const DEBOUNCE_MS = 220;
const MIN_CHARS = 2;
const LRU_CAP = 40;

const normalizeKey = (query: string) => query.trim().replace(/\s+/g, ' ').toLowerCase();

class SuggestLru {
  private order: string[] = [];
  private entries = new Map<string, SearchSuggestItem[]>();

  get(key: string): SearchSuggestItem[] | undefined {
    const hit = this.entries.get(key);
    if (!hit) return undefined;
    this.touch(key);
    return hit;
  }

  set(key: string, items: SearchSuggestItem[]) {
    this.entries.set(key, items);
    this.touch(key);
    while (this.order.length > LRU_CAP) {
      const oldest = this.order.shift();
      if (oldest) this.entries.delete(oldest);
    }
  }

  private touch(key: string) {
    const index = this.order.indexOf(key);
    if (index >= 0) this.order.splice(index, 1);
    this.order.push(key);
  }
}

const lru = new SuggestLru();

/** Debounced Steam suggest with a session LRU so backspace is instant. */
export const useSteamSuggest = (query: string, enabled: boolean) => {
  const [items, setItems] = useState<SearchSuggestItem[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const requestId = useRef(0);

  useEffect(() => {
    const trimmed = query.trim();
    if (!enabled || trimmed.length < MIN_CHARS) {
      requestId.current += 1;
      setIsLoading(false);
      if (!enabled || trimmed.length === 0) {
        setItems([]);
      }
      return;
    }

    const key = normalizeKey(trimmed);
    const cached = lru.get(key);
    if (cached) {
      requestId.current += 1;
      setItems(cached);
      setIsLoading(false);
      return;
    }

    const current = requestId.current + 1;
    requestId.current = current;
    setIsLoading(true);

    const timer = window.setTimeout(async () => {
      try {
        const results: SearchSuggestItem[] = await invoke('suggest_store_games', { query: trimmed });
        if (requestId.current !== current) return;
        const next = results || [];
        lru.set(key, next);
        setItems(next);
      } catch (err) {
        if (requestId.current !== current) return;
        console.warn('[store] suggest failed:', err);
      } finally {
        if (requestId.current === current) {
          setIsLoading(false);
        }
      }
    }, DEBOUNCE_MS);

    return () => {
      window.clearTimeout(timer);
    };
  }, [query, enabled]);

  return { items, isLoading };
};
